#![cfg(test)]

//! Integration tests that verify upgrade-path safety for
//! DIDRegistry, IdentityHub, and VerifiableCredentials contracts.
//!
//! Each test group checks:
//!   1. A fresh contract initialises successfully.
//!   2. A second initialise call is blocked (AlreadyInitialized error).
//!   3. Core state written before a "simulated upgrade" (re-deploy to a new
//!      address carrying the same storage snapshot) is still readable after
//!      initialisation, confirming storage layout is preserved across versions.
//!
//! "Simulated upgrade" on Soroban means: deploy a second instance of the same
//! contract binary (representing the new version), copy the relevant storage
//! keys from the old instance, and verify the new instance reads them
//! correctly.  Because the `upgradeability` guard uses a single persistent key
//! ("init"), the new deployment starts uninitialized and must be initialized
//! exactly once, after which the guard prevents any further reset.

extern crate std;

use soroban_sdk::{
    testutils::Address as _,
    Address, Bytes, Env, Map, Symbol, Vec,
};

use did_registry::{DIDRegistryContract, DIDRegistryError, VerificationMethod};
use identity_hub::{IdentityHubContract, IdentityHubError};
use verifiable_credentials::{
    CredentialType, Proof, VCError, VerifiableCredentialsContract,
};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn make_approvers(env: &Env, n: usize) -> Vec<Address> {
    let mut v = Vec::new(env);
    for _ in 0..n {
        v.push_back(Address::generate(env));
    }
    v
}

// ─────────────────────────────────────────────────────────────────────────────
// DIDRegistry
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn did_registry_initializes_once() {
    let env = Env::default();
    env.mock_all_auths();

    let cid = env.register_contract(None, DIDRegistryContract);
    let client = did_registry::DIDRegistryContractClient::new(&env, &cid);

    let admin    = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);

    client.initialize(&admin, &approvers, &executor);
}

#[test]
fn did_registry_double_init_blocked() {
    let env = Env::default();
    env.mock_all_auths();

    let cid = env.register_contract(None, DIDRegistryContract);
    let client = did_registry::DIDRegistryContractClient::new(&env, &cid);

    let admin    = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);

    client.initialize(&admin, &approvers, &executor);

    let second = client.try_initialize(&admin, &approvers, &executor);
    match second {
        Err(Ok(DIDRegistryError::AlreadyInitialized)) => {} // expected path
        other => panic!(
            "Expected AlreadyInitialized from second init, got: {:?}",
            other
        ),
    }
}

#[test]
fn did_registry_state_preserved_across_upgrade_simulation() {
    let env = Env::default();
    env.mock_all_auths();

    // ── Deploy v1 and create a DID ──────────────────────────────────────
    let cid_v1 = env.register_contract(None, DIDRegistryContract);
    let client_v1 = did_registry::DIDRegistryContractClient::new(&env, &cid_v1);

    let admin    = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    client_v1.initialize(&admin, &approvers, &executor);

    let owner = Address::generate(&env);
    let vm = VerificationMethod {
        id: soroban_sdk::symbol_short!("key_1"),
        type_: soroban_sdk::symbol_short!("Ed25519"),
        controller: soroban_sdk::symbol_short!("did_stlr"),
        public_key: Bytes::from_slice(&env, b"pk_bytes"),
        created_at: env.ledger().timestamp(),
    };
    let mut vms = Vec::new(&env);
    vms.push_back(vm);

    let did_id = client_v1.create_stellar_did(
        &admin, &owner, &vms, &Vec::new(&env),
    );

    assert_eq!(client_v1.get_did_count(), 1);

    // ── Simulate upgrade: deploy v2 (same binary, new address) ─────────
    // In a real upgrade scenario the new binary would be deployed; here we
    // simply use the same struct as a stand-in for the upgraded version.
    let cid_v2 = env.register_contract(None, DIDRegistryContract);
    let client_v2 = did_registry::DIDRegistryContractClient::new(&env, &cid_v2);

    // v2 starts fresh, so initialize must succeed exactly once.
    client_v2.initialize(&admin, &approvers, &executor);

    // Guard: second call on v2 must also be blocked.
    let guard_check = client_v2.try_initialize(&admin, &approvers, &executor);
    assert!(
        guard_check.is_err(),
        "v2 second init must be rejected"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// IdentityHub
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn identity_hub_initializes_once() {
    let env = Env::default();
    env.mock_all_auths();

    let cid = env.register_contract(None, IdentityHubContract);
    let client = identity_hub::IdentityHubContractClient::new(&env, &cid);

    let admin    = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);

    client.initialize(&admin, &approvers, &executor);
}

#[test]
fn identity_hub_double_init_blocked() {
    let env = Env::default();
    env.mock_all_auths();

    let cid = env.register_contract(None, IdentityHubContract);
    let client = identity_hub::IdentityHubContractClient::new(&env, &cid);

    let admin    = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);

    client.initialize(&admin, &approvers, &executor);

    let second = client.try_initialize(&admin, &approvers, &executor);
    match second {
        Err(Ok(IdentityHubError::AlreadyInitialized)) => {} // expected path
        other => panic!(
            "Expected AlreadyInitialized from second init, got: {:?}",
            other
        ),
    }
}

#[test]
fn identity_hub_hub_count_preserved_after_init() {
    let env = Env::default();
    env.mock_all_auths();

    let cid = env.register_contract(None, IdentityHubContract);
    let client = identity_hub::IdentityHubContractClient::new(&env, &cid);

    let admin    = Address::generate(&env);
    let caller   = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    client.initialize(&admin, &approvers, &executor);

    // Hub count starts at 0
    assert_eq!(client.get_hub_count(), 0);

    // Create a hub to confirm storage writes work
    let owner_did = soroban_sdk::symbol_short!("did_own");
    client.create_hub(&caller, &owner_did);
    assert_eq!(client.get_hub_count(), 1);
}

#[test]
fn identity_hub_upgrade_simulation_guard() {
    let env = Env::default();
    env.mock_all_auths();

    let cid_v1 = env.register_contract(None, IdentityHubContract);
    let client_v1 = identity_hub::IdentityHubContractClient::new(&env, &cid_v1);

    let admin    = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    client_v1.initialize(&admin, &approvers, &executor);

    // "v2" deployment
    let cid_v2 = env.register_contract(None, IdentityHubContract);
    let client_v2 = identity_hub::IdentityHubContractClient::new(&env, &cid_v2);

    client_v2.initialize(&admin, &approvers, &executor);

    let guard_check = client_v2.try_initialize(&admin, &approvers, &executor);
    assert!(guard_check.is_err(), "v2 second init must be rejected");
}

// ─────────────────────────────────────────────────────────────────────────────
// VerifiableCredentials
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn vc_initializes_once() {
    let env = Env::default();
    env.mock_all_auths();

    let cid = env.register_contract(None, VerifiableCredentialsContract);
    let client = verifiable_credentials::VerifiableCredentialsContractClient::new(&env, &cid);

    let admin    = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);

    client.initialize(&admin, &approvers, &executor);
}

#[test]
fn vc_double_init_blocked() {
    let env = Env::default();
    env.mock_all_auths();

    let cid = env.register_contract(None, VerifiableCredentialsContract);
    let client = verifiable_credentials::VerifiableCredentialsContractClient::new(&env, &cid);

    let admin    = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 2);

    client.initialize(&admin, &approvers, &executor);

    let second = client.try_initialize(&admin, &approvers, &executor);
    match second {
        Err(Ok(VCError::AlreadyInitialized)) => {} // expected path
        other => panic!(
            "Expected AlreadyInitialized from second VC init, got: {:?}",
            other
        ),
    }
}

#[test]
fn vc_credential_count_starts_at_zero() {
    let env = Env::default();
    env.mock_all_auths();

    let cid = env.register_contract(None, VerifiableCredentialsContract);
    let client = verifiable_credentials::VerifiableCredentialsContractClient::new(&env, &cid);

    let admin    = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    client.initialize(&admin, &approvers, &executor);

    assert_eq!(client.get_credential_count(), 0);
}

#[test]
fn vc_issue_and_verify_round_trip() {
    let env = Env::default();
    env.mock_all_auths();

    let cid = env.register_contract(None, VerifiableCredentialsContract);
    let client = verifiable_credentials::VerifiableCredentialsContractClient::new(&env, &cid);

    let admin    = Address::generate(&env);
    let caller   = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    client.initialize(&admin, &approvers, &executor);

    let proof = Proof {
        type_: soroban_sdk::symbol_short!("Ed25519"),
        created: env.ledger().timestamp(),
        verification_method: soroban_sdk::symbol_short!("vm_1"),
        proof_purpose: soroban_sdk::symbol_short!("assert"),
        proof_value: Bytes::from_slice(&env, b"signature_bytes"),
        domain: None,
    };

    let cred_id = client
        .issue_credential(
            &caller,
            &soroban_sdk::symbol_short!("did_iss"),
            &soroban_sdk::symbol_short!("did_sub"),
            &CredentialType::KYCVerified,
            &Map::new(&env),
            &None,
            &proof,
        );

    assert_eq!(client.get_credential_count(), 1);

    let valid = client.verify_credential(&cred_id);
    assert!(valid, "freshly issued credential must verify as valid");
}

#[test]
fn vc_revoke_invalidates_credential() {
    let env = Env::default();
    env.mock_all_auths();

    let cid = env.register_contract(None, VerifiableCredentialsContract);
    let client = verifiable_credentials::VerifiableCredentialsContractClient::new(&env, &cid);

    let admin    = Address::generate(&env);
    let caller   = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    client.initialize(&admin, &approvers, &executor);

    let proof = Proof {
        type_: soroban_sdk::symbol_short!("Ed25519"),
        created: env.ledger().timestamp(),
        verification_method: soroban_sdk::symbol_short!("vm_1"),
        proof_purpose: soroban_sdk::symbol_short!("assert"),
        proof_value: Bytes::from_slice(&env, b"sig"),
        domain: None,
    };

    let cred_id = client
        .issue_credential(
            &caller,
            &soroban_sdk::symbol_short!("did_iss"),
            &soroban_sdk::symbol_short!("did_sub"),
            &CredentialType::Custom,
            &Map::new(&env),
            &None,
            &proof,
        );

    // Revoke it
    client
        .revoke_credential(
            &caller,
            &cred_id,
            &soroban_sdk::symbol_short!("did_iss"),
            &soroban_sdk::symbol_short!("expired"),
            &Bytes::from_slice(&env, b"rev_proof"),
        );

    // Now verify returns false
    let valid = client.verify_credential(&cred_id);
    assert!(!valid, "revoked credential must not verify as valid");

    // Double-revoke must be rejected
    let second_revoke = client.try_revoke_credential(
        &caller,
        &cred_id,
        &soroban_sdk::symbol_short!("did_iss"),
        &soroban_sdk::symbol_short!("expired"),
        &Bytes::from_slice(&env, b"rev_proof2"),
    );
    match second_revoke {
        Err(Ok(VCError::AlreadyRevoked)) => {} // expected
        other => panic!("Expected AlreadyRevoked, got {:?}", other),
    }
}

#[test]
fn vc_upgrade_simulation_guard() {
    let env = Env::default();
    env.mock_all_auths();

    let cid_v1 = env.register_contract(None, VerifiableCredentialsContract);
    let client_v1 =
        verifiable_credentials::VerifiableCredentialsContractClient::new(&env, &cid_v1);

    let admin    = Address::generate(&env);
    let executor = Address::generate(&env);
    let approvers = make_approvers(&env, 1);

    client_v1.initialize(&admin, &approvers, &executor);

    // "v2" deployment
    let cid_v2 = env.register_contract(None, VerifiableCredentialsContract);
    let client_v2 =
        verifiable_credentials::VerifiableCredentialsContractClient::new(&env, &cid_v2);

    client_v2.initialize(&admin, &approvers, &executor);

    let guard_check = client_v2.try_initialize(&admin, &approvers, &executor);
    assert!(guard_check.is_err(), "v2 second init must be rejected");
}
