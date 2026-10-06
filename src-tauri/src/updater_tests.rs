use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use tauri_plugin_updater::UpdaterExt;

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

#[tokio::test]
async fn real_updater_download_verifies_the_artifact_and_rejects_tampering_and_version_mismatch() {
    use crate::updates::Package;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };

    let configuration: serde_json::Value =
        serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    let original = include_bytes!("../tests/fixtures/updater-check.txt");
    for scenario in ["valid", "tampered", "wrong-version"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let version = if scenario == "wrong-version" {
            "99.0.0"
        } else {
            "0.0.0-test"
        };
        let manifest = serde_json::to_vec(&serde_json::json!({
            "version": version,
            "platforms": { "test": {
                "url": format!("http://{address}/artifact"),
                "signature": include_str!("../tests/fixtures/updater-check.txt.sig").trim(),
            } }
        }))
        .unwrap();
        let mut artifact = original.to_vec();
        if scenario == "tampered" {
            artifact[0] ^= 1;
        }
        let server = std::thread::spawn(move || {
            for body in [manifest, artifact] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut buffer = [0; 1024];
                    let read = stream.read(&mut buffer).unwrap();
                    assert_ne!(read, 0);
                    request.extend_from_slice(&buffer[..read]);
                }
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                )
                .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().plugins.0.insert(
            "updater".into(),
            serde_json::json!({
                "pubkey": configuration["plugins"]["updater"]["pubkey"],
                "requireSignedVersion": true,
                // HTTP is confined to this loopback fixture, never the production configuration.
                "dangerousInsecureTransportProtocol": true,
            }),
        );
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .unwrap();
        let update = app
            .updater_builder()
            .target("test")
            .endpoints(vec![format!("http://{address}/feed").parse().unwrap()])
            .unwrap()
            .version_comparator(|_, _| true)
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
            .check()
            .await
            .unwrap()
            .unwrap();
        let result = Package::download(&update, |_| {}).await;
        if scenario == "valid" {
            assert_eq!(result.unwrap(), original);
        } else {
            assert!(result.is_err(), "{scenario} was accepted");
        }
        server.join().unwrap();
    }
}
