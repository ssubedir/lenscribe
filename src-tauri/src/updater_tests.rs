use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};

#[test]
fn configured_updater_key_verifies_the_signed_fixture_and_rejects_tampering() {
    let configuration: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let settings = &configuration["plugins"]["updater"];
    assert_eq!(settings["requireSignedVersion"], true);
    let public_key = String::from_utf8(
        STANDARD
            .decode(settings["pubkey"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    let signature = String::from_utf8(
        STANDARD
            .decode(include_str!("../tests/fixtures/updater-check.txt.sig").trim())
            .unwrap(),
    )
    .unwrap();
    let public_key = PublicKey::decode(&public_key).unwrap();
    let signature = Signature::decode(&signature).unwrap();
    let artifact = include_bytes!("../tests/fixtures/updater-check.txt");
    public_key.verify(artifact, &signature, true).unwrap();
    assert!(signature.trusted_comment().contains("0.0.0-test"));
    let mut changed = artifact.to_vec();
    changed[0] ^= 1;
    assert!(public_key.verify(&changed, &signature, true).is_err());
}
