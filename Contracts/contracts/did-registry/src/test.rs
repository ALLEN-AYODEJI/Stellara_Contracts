extern crate std;

use soroban_sdk::{
    symbol_short,
    testutils::Address as _,
    Address, Bytes, Env, Vec,
};

use crate::{DIDRegistryContract, DIDRegistryError, DIDRegistryContractClient, Service, VerificationMethod};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn register_and_init(env: &Env) -> (DIDRegistryContractClient, Address) {
    env.mock_all_auths();
    let contract_id = env.register_contract(None, DIDRegistryContract);
    let client = DIDRegistryContractClient::new(env, &contract_id);

    let admin    = Address::generate(env);
    let approver = Address::generate(env);
    let executor = Address::generate(env);

    let mut approvers = Vec::new(env);
    approvers.push_back(approver);

    // The generated client panics on error; success returns ()
    client.initialize(&admin, &approvers, &executor);

    (client, admin)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn test_initialize() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, DIDRegistryContract);
    let client = DIDRegistryContractClient::new(&env, &contract_id);

    let admin    = Address::generate(&env);
    let approver = Address::generate(&env);
    let executor = Address::generate(&env);
    let mut approvers = Vec::new(&env);
    approvers.push_back(approver);

    // First init: generated client panics on error, returns () on success
    client.initialize(&admin, &approvers, &executor);
    assert_eq!(client.get_did_count(), 0);
}

#[test]
fn test_double_initialize_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register_contract(None, DIDRegistryContract);
    let client = DIDRegistryContractClient::new(&env, &contract_id);

    let admin    = Address::generate(&env);
    let approver = Address::generate(&env);
    let executor = Address::generate(&env);
    let mut approvers = Vec::new(&env);
    approvers.push_back(approver);

    client.initialize(&admin, &approvers, &executor);

    // try_initialize returns Result<Result<(),VCError>, sdk::Error>
    let result = client.try_initialize(&admin, &approvers, &executor);
    match result {
        Err(Ok(DIDRegistryError::AlreadyInitialized)) => {}
        other => panic!("Expected AlreadyInitialized, got {:?}", other),
    }
}

#[test]
fn test_create_stellar_did() {
    let env = Env::default();
    let (client, admin) = register_and_init(&env);

    let owner = Address::generate(&env);
    let mut vms = Vec::new(&env);
    vms.push_back(VerificationMethod {
        id: symbol_short!("key1"),
        type_: symbol_short!("Ed25519"),
        controller: symbol_short!("did"),
        public_key: Bytes::from_slice(&env, b"pk"),
        created_at: 0,
    });

    let did_id = client.create_stellar_did(&admin, &owner, &vms, &Vec::new(&env));

    assert_eq!(client.get_did_count(), 1);
    let doc = client.resolve_did(&did_id);
    assert_eq!(doc.id, did_id);
    assert_eq!(doc.verification_methods.len(), 1);
    assert!(!doc.deactivated);
    assert_eq!(doc.owner, owner);
}

#[test]
fn test_create_key_did() {
    let env = Env::default();
    let (client, admin) = register_and_init(&env);

    let pk = Bytes::from_slice(&env, b"z6MkhaXgBZDvotDkL5257");
    let owner = Address::generate(&env);

    let did_id = client.create_key_did(&admin, &pk, &owner, &Vec::new(&env), &Vec::new(&env));

    assert_eq!(client.get_did_count(), 1);
    let doc = client.resolve_did(&did_id);
    assert!(!doc.deactivated);
    assert_eq!(doc.owner, owner);
}

#[test]
fn test_add_verification_method() {
    let env = Env::default();
    let (client, admin) = register_and_init(&env);

    let owner = Address::generate(&env);
    let did_id = client.create_stellar_did(&admin, &owner, &Vec::new(&env), &Vec::new(&env));

    client.add_verification_method(&owner, &did_id, &VerificationMethod {
        id: symbol_short!("key2"),
        type_: symbol_short!("Ed25519"),
        controller: symbol_short!("did"),
        public_key: Bytes::from_slice(&env, b"newpk"),
        created_at: 0,
    });

    let doc = client.resolve_did(&did_id);
    assert_eq!(doc.verification_methods.len(), 1);
    assert_eq!(doc.authentication.len(), 1);
}

#[test]
fn test_add_service() {
    let env = Env::default();
    let (client, admin) = register_and_init(&env);

    let owner = Address::generate(&env);
    let did_id = client.create_stellar_did(&admin, &owner, &Vec::new(&env), &Vec::new(&env));

    client.add_service(&owner, &did_id, &Service {
        id: symbol_short!("hub1"),
        type_: symbol_short!("IdHub"),
        service_endpoint: symbol_short!("https"),
        created_at: 0,
    });

    let doc = client.resolve_did(&did_id);
    assert_eq!(doc.service.len(), 1);
    assert_eq!(doc.service.get(0).unwrap().id, symbol_short!("hub1"));
}

#[test]
fn test_deactivate_did() {
    let env = Env::default();
    let (client, admin) = register_and_init(&env);

    let owner = Address::generate(&env);
    let did_id = client.create_stellar_did(&admin, &owner, &Vec::new(&env), &Vec::new(&env));

    client.deactivate_did(&owner, &did_id);

    assert!(client.resolve_did(&did_id).deactivated);
}

#[test]
fn test_get_all_dids() {
    let env = Env::default();
    let (client, admin) = register_and_init(&env);

    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    client.create_stellar_did(&admin, &a1, &Vec::new(&env), &Vec::new(&env));
    client.create_stellar_did(&admin, &a2, &Vec::new(&env), &Vec::new(&env));

    assert_eq!(client.get_all_dids(&admin).len(), 2);
}
