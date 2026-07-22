#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    Address, Bytes, Env, Map, Symbol, Vec,
    contracterror,
};
use shared::acl::{ACL, PERMISSION_PROPOSE};
use shared::governance::{GovernanceManager, GovernanceRole};
use shared::events::{
    extended_topics,
    DidCreatedEvent, DidUpdatedEvent, DidDeactivatedEvent,
    VerificationMethodAddedEvent, ServiceAddedEvent,
};

// ─────────────────────────────────────────────────────────────────────────────
// Storage keys
// ─────────────────────────────────────────────────────────────────────────────

mod keys {
    use soroban_sdk::{symbol_short, Symbol};
    pub const DIDS:    Symbol = symbol_short!("dids");
    pub const DID_CNT: Symbol = symbol_short!("did_cnt");
    pub const ADDRMAP: Symbol = symbol_short!("addr_did");
}

// ─────────────────────────────────────────────────────────────────────────────
// Data structures
// ─────────────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DIDDocument {
    pub id: Symbol,
    pub verification_methods: Vec<VerificationMethod>,
    pub authentication: Vec<Symbol>,
    pub assertion_method: Vec<Symbol>,
    pub key_agreement: Vec<Symbol>,
    pub service: Vec<Service>,
    pub created_at: u64,
    pub updated_at: u64,
    pub deactivated: bool,
    pub owner: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationMethod {
    pub id: Symbol,
    pub type_: Symbol,
    pub controller: Symbol,
    pub public_key: Bytes,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Service {
    pub id: Symbol,
    pub type_: Symbol,
    pub service_endpoint: Symbol,
    pub created_at: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Error codes
// ─────────────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DIDRegistryError {
    InvalidDIDFormat          = 3001,
    DIDNotFound               = 3002,
    Unauthorized              = 3003,
    InvalidVerificationMethod = 3004,
    DuplicateService          = 3005,
    AlreadyDeactivated        = 3006,
    InvalidPublicKey          = 3007,
    GovernanceError           = 3008,
    /// Returned when `initialize()` is called more than once.
    AlreadyInitialized        = 3009,
}

// ─────────────────────────────────────────────────────────────────────────────
// Contract
// ─────────────────────────────────────────────────────────────────────────────

#[contract]
pub struct DIDRegistryContract;

#[contractimpl]
impl DIDRegistryContract {
    // ── Initialization ────────────────────────────────────────────────────

    /// Initialize the contract with ACL-backed governance roles.
    ///
    /// Protected by the upgradeability module — a second call returns
    /// `AlreadyInitialized` without touching any state.
    pub fn initialize(
        env: Env,
        admin: Address,
        approvers: Vec<Address>,
        executor: Address,
    ) -> Result<(), DIDRegistryError> {
        if upgradeability::is_initialized(&env) {
            return Err(DIDRegistryError::AlreadyInitialized);
        }
        upgradeability::mark_initialized(&env);

        // Wire up ACL-backed governance (used by GovernanceManager::require_role)
        GovernanceManager::init_governance_roles(&env, admin, approvers, executor);
        env.storage().persistent().set(&keys::DID_CNT, &0u64);

        Ok(())
    }

    // ── DID creation ──────────────────────────────────────────────────────

    pub fn create_stellar_did(
        env: Env,
        caller: Address,
        stellar_address: Address,
        verification_methods: Vec<VerificationMethod>,
        services: Vec<Service>,
    ) -> Symbol {
        caller.require_auth();
        GovernanceManager::require_role(&env, &caller, GovernanceRole::Admin);

        // Derive a deterministic DID symbol from the address using the counter
        let did_id = Self::address_to_did_symbol(&env, &stellar_address);

        if Self::load_document(&env, &did_id).is_some() {
            panic!("DID already exists");
        }

        let mut authentication = Vec::new(&env);
        for vm in verification_methods.iter() {
            authentication.push_back(vm.id.clone());
        }

        Self::save_document(&env, &did_id, DIDDocument {
            id: did_id.clone(),
            verification_methods,
            authentication,
            assertion_method: Vec::new(&env),
            key_agreement: Vec::new(&env),
            service: services,
            created_at: env.ledger().timestamp(),
            updated_at: env.ledger().timestamp(),
            deactivated: false,
            owner: stellar_address.clone(),
        });
        Self::inc_counter(&env);

        env.events().publish(
            (extended_topics::DID_CREATED,),
            DidCreatedEvent {
                did: did_id.clone(),
                controller: stellar_address,
                method: symbol_short!("stellar"),
                timestamp: env.ledger().timestamp(),
            },
        );
        did_id
    }

    pub fn create_key_did(
        env: Env,
        caller: Address,
        public_key: Bytes,
        owner: Address,
        verification_methods: Vec<VerificationMethod>,
        services: Vec<Service>,
    ) -> Symbol {
        caller.require_auth();
        GovernanceManager::require_role(&env, &caller, GovernanceRole::Admin);

        if public_key.is_empty() {
            panic!("Invalid public key");
        }

        let count: u64 = env.storage().persistent().get(&keys::DID_CNT).unwrap_or(0);
        let did_id = Self::count_symbol(&env, count, "dk");

        if Self::load_document(&env, &did_id).is_some() {
            panic!("DID already exists");
        }

        let mut authentication = Vec::new(&env);
        for vm in verification_methods.iter() {
            authentication.push_back(vm.id.clone());
        }

        Self::save_document(&env, &did_id, DIDDocument {
            id: did_id.clone(),
            verification_methods,
            authentication,
            assertion_method: Vec::new(&env),
            key_agreement: Vec::new(&env),
            service: services,
            created_at: env.ledger().timestamp(),
            updated_at: env.ledger().timestamp(),
            deactivated: false,
            owner,
        });
        Self::inc_counter(&env);

        env.events().publish(
            (extended_topics::DID_CREATED,),
            DidCreatedEvent {
                did: did_id.clone(),
                controller: env.current_contract_address(),
                method: symbol_short!("key"),
                timestamp: env.ledger().timestamp(),
            },
        );
        did_id
    }

    // ── Resolution ────────────────────────────────────────────────────────

    pub fn resolve_did(env: Env, did: Symbol) -> DIDDocument {
        Self::load_document(&env, &did).unwrap_or_else(|| panic!("DID not found"))
    }

    // ── Mutation ──────────────────────────────────────────────────────────

    pub fn update_did_document(
        env: Env,
        caller: Address,
        did: Symbol,
        verification_methods: Option<Vec<VerificationMethod>>,
        services: Option<Vec<Service>>,
    ) {
        caller.require_auth();
        let mut doc = Self::load_document(&env, &did).unwrap_or_else(|| panic!("DID not found"));

        if !Self::is_authorized(&env, &caller, &doc) { panic!("Unauthorized"); }
        if doc.deactivated { panic!("DID is deactivated"); }

        if let Some(vms) = verification_methods {
            doc.authentication = Vec::new(&env);
            for vm in vms.iter() { doc.authentication.push_back(vm.id.clone()); }
            doc.verification_methods = vms;
        }
        if let Some(svcs) = services { doc.service = svcs; }
        doc.updated_at = env.ledger().timestamp();
        Self::save_document(&env, &did, doc);

        env.events().publish(
            (extended_topics::DID_UPDATED,),
            DidUpdatedEvent { did, controller: env.current_contract_address(), timestamp: env.ledger().timestamp() },
        );
    }

    pub fn deactivate_did(env: Env, caller: Address, did: Symbol) {
        caller.require_auth();
        let mut doc = Self::load_document(&env, &did).unwrap_or_else(|| panic!("DID not found"));

        if !Self::is_authorized(&env, &caller, &doc) { panic!("Unauthorized"); }
        if doc.deactivated { panic!("DID already deactivated"); }

        doc.deactivated = true;
        doc.updated_at = env.ledger().timestamp();
        Self::save_document(&env, &did, doc);

        env.events().publish(
            (extended_topics::DID_DEACTIVATED,),
            DidDeactivatedEvent { did, deactivated_by: env.current_contract_address(), timestamp: env.ledger().timestamp() },
        );
    }

    pub fn add_verification_method(
        env: Env,
        caller: Address,
        did: Symbol,
        verification_method: VerificationMethod,
    ) {
        caller.require_auth();
        let mut doc = Self::load_document(&env, &did).unwrap_or_else(|| panic!("DID not found"));

        if !Self::is_authorized(&env, &caller, &doc) { panic!("Unauthorized"); }
        if doc.deactivated { panic!("DID is deactivated"); }
        if doc.verification_methods.iter().any(|vm| vm.id == verification_method.id) {
            panic!("Verification method already exists");
        }

        doc.authentication.push_back(verification_method.id.clone());
        doc.verification_methods.push_back(verification_method.clone());
        doc.updated_at = env.ledger().timestamp();
        Self::save_document(&env, &did, doc);

        env.events().publish(
            (extended_topics::VERIF_METHOD_ADDED,),
            VerificationMethodAddedEvent {
                did,
                method_id: verification_method.id,
                controller: env.current_contract_address(),
                timestamp: env.ledger().timestamp(),
            },
        );
    }

    pub fn add_service(env: Env, caller: Address, did: Symbol, service: Service) {
        caller.require_auth();
        let mut doc = Self::load_document(&env, &did).unwrap_or_else(|| panic!("DID not found"));

        if !Self::is_authorized(&env, &caller, &doc) { panic!("Unauthorized"); }
        if doc.deactivated { panic!("DID is deactivated"); }
        if doc.service.iter().any(|s| s.id == service.id) { panic!("Service already exists"); }

        doc.service.push_back(service.clone());
        doc.updated_at = env.ledger().timestamp();
        Self::save_document(&env, &did, doc);

        env.events().publish(
            (extended_topics::SERVICE_ADDED,),
            ServiceAddedEvent { did, service_id: service.id, controller: env.current_contract_address(), timestamp: env.ledger().timestamp() },
        );
    }

    // ── Queries ───────────────────────────────────────────────────────────

    pub fn get_all_dids(env: Env, caller: Address) -> Vec<Symbol> {
        caller.require_auth();
        GovernanceManager::require_role(&env, &caller, GovernanceRole::Admin);

        let dids: Map<Symbol, DIDDocument> = env.storage().persistent()
            .get(&keys::DIDS).unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (id, _) in dids.iter() { result.push_back(id); }
        result
    }

    pub fn get_did_count(env: Env) -> u64 {
        env.storage().persistent().get(&keys::DID_CNT).unwrap_or(0)
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn is_authorized(env: &Env, caller: &Address, doc: &DIDDocument) -> bool {
        caller == &doc.owner || ACL::has_permission(env, caller, &PERMISSION_PROPOSE)
    }

    fn load_document(env: &Env, did: &Symbol) -> Option<DIDDocument> {
        let dids: Map<Symbol, DIDDocument> = env.storage().persistent()
            .get(&keys::DIDS).unwrap_or_else(|| Map::new(env));
        dids.get(did.clone())
    }

    fn save_document(env: &Env, did: &Symbol, doc: DIDDocument) {
        let mut dids: Map<Symbol, DIDDocument> = env.storage().persistent()
            .get(&keys::DIDS).unwrap_or_else(|| Map::new(env));
        dids.set(did.clone(), doc);
        env.storage().persistent().set(&keys::DIDS, &dids);
    }

    fn inc_counter(env: &Env) {
        let n: u64 = env.storage().persistent().get(&keys::DID_CNT).unwrap_or(0);
        env.storage().persistent().set(&keys::DID_CNT, &(n + 1));
    }

    /// Derive a unique DID symbol for a stellar address, memoised by counter.
    fn address_to_did_symbol(env: &Env, addr: &Address) -> Symbol {
        let mut map: Map<Address, Symbol> = env.storage().persistent()
            .get(&keys::ADDRMAP).unwrap_or_else(|| Map::new(env));
        if let Some(s) = map.get(addr.clone()) { return s; }

        let count: u64 = env.storage().persistent().get(&keys::DID_CNT).unwrap_or(0);
        let s = Self::count_symbol(env, count, "ds");

        map.set(addr.clone(), s.clone());
        env.storage().persistent().set(&keys::ADDRMAP, &map);
        s
    }

    /// Encode a u64 counter into a ≤9-char Symbol using base-36.
    fn count_symbol(env: &Env, count: u64, prefix: &str) -> Symbol {
        let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
        let base = 36u64;
        let mut buf = [b'0'; 9];
        let pb = prefix.as_bytes();
        let plen = pb.len().min(2);
        buf[..plen].copy_from_slice(&pb[..plen]);

        let mut n = count;
        let mut pos = 8usize;
        loop {
            buf[pos] = digits[(n % base) as usize];
            n /= base;
            if n == 0 || pos == plen { break; }
            pos -= 1;
        }

        let start = buf[plen..].iter().position(|&b| b != b'0')
            .map(|p| p + plen).unwrap_or(plen);
        let s = core::str::from_utf8(&buf[start..]).unwrap_or("x0");
        Symbol::new(env, s)
    }
}

#[cfg(test)]
mod test;
