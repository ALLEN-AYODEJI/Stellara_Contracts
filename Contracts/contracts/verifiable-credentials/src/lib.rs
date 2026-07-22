#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    Address, Bytes, Env, Map, Symbol, Vec,
    contracterror,
};
use shared::governance::{GovernanceManager, GovernanceRole};
use shared::events::{extended_topics, CredentialIssuedEvent, CredentialRevokedEvent};

// ─────────────────────────────────────────────────────────────────────────────
// Storage keys
// ─────────────────────────────────────────────────────────────────────────────

mod keys {
    use soroban_sdk::{symbol_short, Symbol};
    pub const ROLES:    Symbol = symbol_short!("roles");
    pub const CREDS:    Symbol = symbol_short!("creds");
    pub const REVOCATN: Symbol = symbol_short!("revocatn");
    pub const VC_CNT:   Symbol = symbol_short!("vc_cnt");
}

// ─────────────────────────────────────────────────────────────────────────────
// Data structures
// ─────────────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiableCredential {
    pub id: Symbol,
    pub context: Symbol,
    pub type_: Vec<Symbol>,
    pub issuer: Symbol,
    pub issuance_date: u64,
    pub expiration_date: Option<u64>,
    pub credential_subject: CredentialSubject,
    pub proof: Proof,
    pub credential_status: CredentialStatus,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialSubject {
    pub id: Symbol,
    pub claims: Map<Symbol, Symbol>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proof {
    pub type_: Symbol,
    pub created: u64,
    pub verification_method: Symbol,
    pub proof_purpose: Symbol,
    pub proof_value: Bytes,
    pub domain: Option<Symbol>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialStatus {
    pub id: Symbol,
    pub type_: Symbol,
    pub status: Symbol,
    pub revocation_reason: Option<Symbol>,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CredentialType {
    KYCVerified            = 0,
    AccreditedInvestor     = 1,
    EducationalAchievement = 2,
    ProfessionalLicense    = 3,
    Custom                 = 4,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevocationEntry {
    pub credential_id: Symbol,
    pub revoker: Symbol,
    pub revocation_date: u64,
    pub reason: Symbol,
    pub proof: Bytes,
}

// ─────────────────────────────────────────────────────────────────────────────
// Error codes
// ─────────────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VCError {
    InvalidCredential   = 4001,
    UnauthorizedIssuer  = 4002,
    CredentialNotFound  = 4003,
    AlreadyRevoked      = 4004,
    ExpiredCredential   = 4005,
    InvalidProof        = 4006,
    InvalidSubject      = 4007,
    GovernanceError     = 4008,
    /// Returned when `initialize()` is called more than once.
    AlreadyInitialized  = 4009,
}

// ─────────────────────────────────────────────────────────────────────────────
// Contract
// ─────────────────────────────────────────────────────────────────────────────

#[contract]
pub struct VerifiableCredentialsContract;

#[contractimpl]
impl VerifiableCredentialsContract {
    // ── Initialization ────────────────────────────────────────────────────

    /// Initialize the contract with governance roles.
    ///
    /// Protected by the upgradeability module — a second call returns
    /// `AlreadyInitialized` without touching any state.
    pub fn initialize(
        env: Env,
        admin: Address,
        approvers: Vec<Address>,
        executor: Address,
    ) -> Result<(), VCError> {
        if upgradeability::is_initialized(&env) {
            return Err(VCError::AlreadyInitialized);
        }
        upgradeability::mark_initialized(&env);

        let mut role_map: Map<Address, GovernanceRole> = Map::new(&env);
        role_map.set(admin.clone(), GovernanceRole::Admin);
        for approver in approvers.iter() {
            role_map.set(approver.clone(), GovernanceRole::Approver);
        }
        role_map.set(executor, GovernanceRole::Executor);

        env.storage().persistent().set(&keys::ROLES, &role_map);
        env.storage().persistent().set(&keys::VC_CNT, &0u64);

        // Initialise empty revocation registry
        let revocations: Map<Symbol, RevocationEntry> = Map::new(&env);
        env.storage().persistent().set(&keys::REVOCATN, &revocations);

        Ok(())
    }

    // ── Issuance ──────────────────────────────────────────────────────────

    pub fn issue_credential(
        env: Env,
        caller: Address,
        issuer_did: Symbol,
        subject_did: Symbol,
        credential_type: CredentialType,
        claims: Map<Symbol, Symbol>,
        expiration_date: Option<u64>,
        proof: Proof,
    ) -> Result<Symbol, VCError> {
        caller.require_auth();

        if proof.proof_value.is_empty() {
            return Err(VCError::InvalidProof);
        }
        if let Some(exp) = expiration_date {
            if exp <= env.ledger().timestamp() {
                return Err(VCError::InvalidCredential);
            }
        }

        let count: u64 = env.storage().persistent().get(&keys::VC_CNT).unwrap_or(0);
        let cred_id = Self::count_symbol(&env, count, "vc");

        let mut type_vec = Vec::new(&env);
        type_vec.push_back(symbol_short!("vc"));
        let type_tag = match credential_type {
            CredentialType::KYCVerified            => symbol_short!("kyc_vc"),
            CredentialType::AccreditedInvestor     => symbol_short!("acc_vc"),
            CredentialType::EducationalAchievement => symbol_short!("edu_vc"),
            CredentialType::ProfessionalLicense    => symbol_short!("pro_vc"),
            CredentialType::Custom                 => symbol_short!("cust_vc"),
        };
        type_vec.push_back(type_tag.clone());

        let status_id = Self::count_symbol(&env, count, "sts");
        let credential = VerifiableCredential {
            id: cred_id.clone(),
            context: symbol_short!("w3c_ctx"),
            type_: type_vec,
            issuer: issuer_did.clone(),
            issuance_date: env.ledger().timestamp(),
            expiration_date,
            credential_subject: CredentialSubject {
                id: subject_did.clone(),
                claims,
            },
            proof,
            credential_status: CredentialStatus {
                id: status_id,
                type_: symbol_short!("csl2021"),
                status: symbol_short!("valid"),
                revocation_reason: None,
            },
            created_at: env.ledger().timestamp(),
        };

        let mut creds: Map<Symbol, VerifiableCredential> = env
            .storage().persistent().get(&keys::CREDS)
            .unwrap_or_else(|| Map::new(&env));
        creds.set(cred_id.clone(), credential);
        env.storage().persistent().set(&keys::CREDS, &creds);
        env.storage().persistent().set(&keys::VC_CNT, &(count + 1));

        env.events().publish(
            (extended_topics::CREDENTIAL_ISSUED,),
            CredentialIssuedEvent {
                credential_id: cred_id.clone(),
                issuer_did,
                subject_did,
                credential_type: type_tag,
                timestamp: env.ledger().timestamp(),
            },
        );

        Ok(cred_id)
    }

    // ── Verification ──────────────────────────────────────────────────────

    pub fn verify_credential(env: Env, credential_id: Symbol) -> Result<bool, VCError> {
        let credential = Self::load_cred(&env, &credential_id)?;

        if Self::is_revoked(&env, &credential_id) {
            return Ok(false);
        }
        if let Some(exp) = credential.expiration_date {
            if env.ledger().timestamp() > exp {
                return Ok(false);
            }
        }
        if credential.proof.proof_value.is_empty() {
            return Err(VCError::InvalidProof);
        }

        Ok(true)
    }

    // ── Revocation ────────────────────────────────────────────────────────

    pub fn revoke_credential(
        env: Env,
        caller: Address,
        credential_id: Symbol,
        revoker_did: Symbol,
        reason: Symbol,
        proof: Bytes,
    ) -> Result<(), VCError> {
        caller.require_auth();

        let mut credential = Self::load_cred(&env, &credential_id)?;

        if Self::is_revoked(&env, &credential_id) {
            return Err(VCError::AlreadyRevoked);
        }

        // Record revocation
        let mut revocations: Map<Symbol, RevocationEntry> = env
            .storage().persistent().get(&keys::REVOCATN)
            .unwrap_or_else(|| Map::new(&env));
        revocations.set(credential_id.clone(), RevocationEntry {
            credential_id: credential_id.clone(),
            revoker: revoker_did,
            revocation_date: env.ledger().timestamp(),
            reason: reason.clone(),
            proof,
        });
        env.storage().persistent().set(&keys::REVOCATN, &revocations);

        // Update status in the credential record
        credential.credential_status.status = symbol_short!("revoked");
        credential.credential_status.revocation_reason = Some(reason.clone());

        let mut creds: Map<Symbol, VerifiableCredential> = env
            .storage().persistent().get(&keys::CREDS)
            .unwrap_or_else(|| Map::new(&env));
        creds.set(credential_id.clone(), credential);
        env.storage().persistent().set(&keys::CREDS, &creds);

        env.events().publish(
            (extended_topics::CREDENTIAL_REVOKED,),
            CredentialRevokedEvent {
                credential_id,
                revoked_by: caller,
                reason,
                timestamp: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    // ── Queries ───────────────────────────────────────────────────────────

    pub fn get_credential_details(env: Env, credential_id: Symbol) -> VerifiableCredential {
        Self::load_cred(&env, &credential_id).unwrap()
    }

    pub fn get_credentials_by_subject(env: Env, subject_did: Symbol) -> Vec<Symbol> {
        let creds: Map<Symbol, VerifiableCredential> = env
            .storage().persistent().get(&keys::CREDS)
            .unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (id, cred) in creds.iter() {
            if cred.credential_subject.id == subject_did {
                result.push_back(id);
            }
        }
        result
    }

    pub fn get_credentials_by_issuer(env: Env, issuer_did: Symbol) -> Vec<Symbol> {
        let creds: Map<Symbol, VerifiableCredential> = env
            .storage().persistent().get(&keys::CREDS)
            .unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (id, cred) in creds.iter() {
            if cred.issuer == issuer_did {
                result.push_back(id);
            }
        }
        result
    }

    pub fn get_revocation_status(env: Env, credential_id: Symbol) -> Option<RevocationEntry> {
        let revocations: Map<Symbol, RevocationEntry> = env
            .storage().persistent().get(&keys::REVOCATN)
            .unwrap_or_else(|| Map::new(&env));
        revocations.get(credential_id)
    }

    pub fn get_credential_count(env: Env) -> u64 {
        env.storage().persistent().get(&keys::VC_CNT).unwrap_or(0)
    }

    pub fn get_all_credentials(env: Env) -> Vec<Symbol> {
        let creds: Map<Symbol, VerifiableCredential> = env
            .storage().persistent().get(&keys::CREDS)
            .unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (id, _) in creds.iter() {
            result.push_back(id);
        }
        result
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn is_revoked(env: &Env, credential_id: &Symbol) -> bool {
        let revocations: Map<Symbol, RevocationEntry> = env
            .storage().persistent().get(&keys::REVOCATN)
            .unwrap_or_else(|| Map::new(env));
        revocations.contains_key(credential_id.clone())
    }

    fn load_cred(env: &Env, credential_id: &Symbol) -> Result<VerifiableCredential, VCError> {
        let creds: Map<Symbol, VerifiableCredential> = env
            .storage().persistent().get(&keys::CREDS)
            .ok_or(VCError::CredentialNotFound)?;
        creds.get(credential_id.clone()).ok_or(VCError::CredentialNotFound)
    }

    /// Build a short unique Symbol from a counter.  Max 9 chars.
    fn count_symbol(env: &Env, count: u64, prefix: &str) -> Symbol {
        let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
        let base = 36u64;
        let mut buf = [0u8; 9];
        let pb = prefix.as_bytes();
        let plen = pb.len().min(2);
        buf[..plen].copy_from_slice(&pb[..plen]);

        let mut n = count;
        let mut pos = 8usize;
        loop {
            buf[pos] = digits[(n % base) as usize];
            n /= base;
            if n == 0 || pos == plen {
                break;
            }
            pos -= 1;
        }

        let start = buf[plen..].iter().position(|&b| b != 0)
            .map(|p| p + plen)
            .unwrap_or(plen);
        let s = core::str::from_utf8(&buf[start..]).unwrap_or("x0");
        Symbol::new(env, s)
    }
}
