use super::*;
use crate::connect::weixin::WeixinClient;
use aes_gcm::aes::cipher::BlockEncryptMut;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

const KEY: &str = "000102030405060708090a0b0c0d0e0f";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nweixin-test-image";

#[tokio::test]
async fn outbound_encrypts_snapshot_and_preserves_protocol_lengths_and_recipient() {
    use crate::connect::outbound::OutboundPolicy;
    let workspace = tempfile::tempdir().unwrap();
    let policy = OutboundPolicy::new(true, "peer", workspace.path().to_str().unwrap());
    let permit = policy.permit("peer").unwrap();
    for (index, (kind, bytes)) in [
        (AttachmentKind::Image, PNG),
        (AttachmentKind::File, b"".as_slice()),
        (AttachmentKind::File, b"0123456789abcdef".as_slice()),
        (AttachmentKind::File, "{\"result\":\"测试\"}".as_bytes()),
        (AttachmentKind::File, b"PK\x03\x04\0\x01\xff".as_slice()),
    ]
    .into_iter()
    .enumerate()
    {
        let server = MockServer::start().await;
        let upload_response = if index % 2 == 0 {
            json!({"upload_full_url":format!("{}/cdn/upload", server.uri())})
        } else {
            json!({"upload_param":"upload-secret"})
        };
        Mock::given(path("/ilink/bot/getuploadurl"))
            .respond_with(ResponseTemplate::new(200).set_body_json(upload_response))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/cdn/upload"))
            .respond_with(
                ResponseTemplate::new(200).insert_header("x-encrypted-param", "download-secret"),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/ilink/bot/sendmessage"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ret":0})))
            .expect(1)
            .mount(&server)
            .await;
        let filename = format!("结果-{index}.bin");
        std::fs::write(workspace.path().join(&filename), bytes).unwrap();
        client(&server)
            .send_workspace_attachment(
                "peer",
                Path::new(&filename),
                kind,
                "context-secret",
                &permit,
            )
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3);
        let request: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(request["to_user_id"], "peer");
        assert_eq!(request["rawsize"], bytes.len());
        assert_eq!(request["rawfilemd5"], hex(&Md5::digest(bytes)));
        assert_eq!(request["filesize"], (bytes.len() / 16 + 1) * 16);
        assert_eq!(
            request["media_type"],
            if kind == AttachmentKind::Image { 1 } else { 3 }
        );
        let key = request["aeskey"].as_str().unwrap();
        assert_eq!(
            decrypt(&requests[1].body, &parse_hex_key(key).unwrap()).unwrap(),
            bytes
        );
        assert_eq!(requests[1].method, "POST");
        for name in [
            "authorization",
            "authorizationtype",
            "x-wechat-uin",
            "x-wechat-route-tag",
            "cookie",
        ] {
            assert!(!requests[1].headers.contains_key(name), "CDN leaked {name}");
        }
        assert_eq!(
            requests[1].headers["content-type"],
            "application/octet-stream"
        );
        let message: serde_json::Value = serde_json::from_slice(&requests[2].body).unwrap();
        assert_eq!(message["msg"]["to_user_id"], "peer");
        assert_eq!(message["msg"]["context_token"], "context-secret");
        let item = &message["msg"]["item_list"][0];
        let payload = if kind == AttachmentKind::Image {
            &item["image_item"]
        } else {
            &item["file_item"]
        };
        assert_eq!(payload["media"]["aes_key"], STANDARD.encode(key));
        assert_eq!(payload["media"]["encrypt_query_param"], "download-secret");
        if kind == AttachmentKind::File {
            assert_eq!(item["type"], 4);
            assert_eq!(payload["file_name"], filename);
            assert_eq!(payload["len"], bytes.len().to_string());
        } else {
            assert_eq!(item["type"], 2);
            assert_eq!(payload["mid_size"], requests[1].body.len());
        }
    }
}

#[tokio::test]
async fn outbound_rejects_upload_failures_and_untrusted_urls_without_sending_or_leaking_keys() {
    use crate::connect::outbound::OutboundPolicy;
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("result.txt"), b"result").unwrap();
    let policy = OutboundPolicy::new(true, "peer", workspace.path().to_str().unwrap());
    let permit = policy.permit("peer").unwrap();
    for (response, cdn_status, header, expected_uploads) in [
        (json!({"ret":-1,"errmsg":"token=secret"}), 200, true, 0),
        (
            json!({"upload_full_url":"https://evil.test/?token=secret"}),
            200,
            true,
            0,
        ),
        (json!({"upload_param":"token=secret"}), 302, true, 1),
        (json!({"upload_param":"token=secret"}), 200, false, 1),
        (json!({}), 200, true, 0),
    ] {
        let server = MockServer::start().await;
        Mock::given(path("/ilink/bot/getuploadurl"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .expect(1)
            .mount(&server)
            .await;
        let mut template = ResponseTemplate::new(cdn_status)
            .insert_header("location", "https://evil.test/?token=secret");
        if header {
            template = template.insert_header("x-encrypted-param", "token=secret");
        }
        Mock::given(path("/cdn/upload"))
            .respond_with(template)
            .expect(expected_uploads)
            .mount(&server)
            .await;
        Mock::given(path("/ilink/bot/sendmessage"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let error = client(&server)
            .send_workspace_attachment(
                "peer",
                Path::new("result.txt"),
                AttachmentKind::File,
                "context",
                &permit,
            )
            .await
            .unwrap_err();
        let error = format!("{error:#}");
        assert!(!error.contains("secret"), "{error}");
        assert!(!error.contains("evil.test"), "{error}");
    }
}

#[tokio::test]
async fn saved_attachment_is_immediately_readable_by_std_fs() {
    let root = tempfile::tempdir().unwrap();
    let dir = Inbox::create(root.path()).unwrap();
    for bytes in [b"report data".as_slice(), PNG, &[42u8; 128 * 1024]] {
        for _ in 0..8 {
            let path = dir.save("attachment.bin", bytes).await.unwrap();
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
    }
}

#[tokio::test]
async fn outbound_revocation_between_http_stages_stops_remaining_requests() {
    use crate::connect::outbound::OutboundPolicy;
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("result.txt"), b"result").unwrap();
    for revoke_at_upload in [false, true] {
        let policy = OutboundPolicy::new(true, "peer", workspace.path().to_str().unwrap());
        let permit = policy.permit("peer").unwrap();
        let server = MockServer::start().await;
        let at_api = policy.clone();
        Mock::given(path("/ilink/bot/getuploadurl"))
            .respond_with(move |_: &wiremock::Request| {
                if !revoke_at_upload {
                    at_api.set(false, "", "");
                }
                ResponseTemplate::new(200).set_body_json(json!({"upload_param":"upload"}))
            })
            .expect(1)
            .mount(&server)
            .await;
        let at_cdn = policy.clone();
        Mock::given(path("/cdn/upload"))
            .respond_with(move |_: &wiremock::Request| {
                at_cdn.set(false, "", "");
                ResponseTemplate::new(200).insert_header("x-encrypted-param", "download")
            })
            .expect(if revoke_at_upload { 1 } else { 0 })
            .mount(&server)
            .await;
        Mock::given(path("/ilink/bot/sendmessage"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let error = client(&server)
            .send_workspace_attachment(
                "peer",
                Path::new("result.txt"),
                AttachmentKind::File,
                "context",
                &permit,
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("权限已改变"));
        assert!(permit.check().is_err());
    }
}

#[tokio::test]
async fn outbound_invalid_binding_and_files_make_no_http_requests() {
    use crate::connect::outbound::OutboundPolicy;
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("result.txt"), b"result").unwrap();
    std::fs::write(workspace.path().join("auth.json"), b"credential").unwrap();
    let policy = OutboundPolicy::new(true, "peer,other", workspace.path().to_str().unwrap());
    let permit = policy.permit("peer").unwrap();
    let server = MockServer::start().await;
    for (peer, path, context) in [
        ("other", "result.txt", "context"),
        ("peer", "result.txt", ""),
        ("peer", "auth.json", "context"),
        ("peer", "missing", "context"),
    ] {
        assert!(
            client(&server)
                .send_workspace_attachment(
                    peer,
                    Path::new(path),
                    AttachmentKind::File,
                    context,
                    &permit
                )
                .await
                .is_err()
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

fn client(server: &MockServer) -> WeixinClient {
    WeixinClient::new(&server.uri(), "bot-secret", "route-secret")
        .unwrap()
        .with_media_base_url(&format!("{}/cdn", server.uri()))
        .unwrap()
}

fn message(items: serde_json::Value) -> WeixinMessage {
    serde_json::from_value(json!({
        "from_user_id": "peer",
        "message_type": 1,
        "message_state": 2,
        "item_list": items,
        "context_token": "context-secret"
    }))
    .unwrap()
}

#[test]
fn aes_ecb_pkcs7_matches_official_node_crypto_for_block_boundaries() {
    // Generated with the official aes-ecb.ts createCipheriv("aes-128-ecb") API.
    let key = parse_hex_key(KEY).unwrap();
    for (plain, expected) in [
        ("", "954f64f2e4e86e9eee82d20216684899"),
        ("hello, weixin", "be8ead8f5e466a5bffe99f774a2d7290"),
        (
            "0123456789abcdef",
            "281567ab2f4cf0d73d3198225b8b8393954f64f2e4e86e9eee82d20216684899",
        ),
        (
            "0123456789abcdefg",
            "281567ab2f4cf0d73d3198225b8b839359d696e1e6c1040eaf9b614dfc82cf8c",
        ),
    ] {
        let ciphertext = encrypt(plain.as_bytes(), &key);
        assert_eq!(hex(&ciphertext), expected);
        assert_eq!(decrypt(&ciphertext, &key).unwrap(), plain.as_bytes());
        assert_eq!(ciphertext.len(), (plain.len() / 16 + 1) * 16);
    }
    assert!(decrypt(&[0; 15], &key).is_err());
    assert!(decrypt(&[], &key).is_err());
}

#[test]
fn accepts_both_official_base64_key_formats_without_echoing_bad_keys() {
    let key = parse_hex_key(KEY).unwrap();
    assert_eq!(parse_base64_key(&STANDARD.encode(key)).unwrap(), key);
    assert_eq!(parse_base64_key(&STANDARD.encode(KEY)).unwrap(), key);
    assert_eq!(parse_hex_key(&KEY.to_uppercase()).unwrap(), key);
    for encoded in ["secret-key-invalid", "token=secret", ""] {
        let error = format!("{:#}", parse_base64_key(encoded).unwrap_err());
        if !encoded.is_empty() {
            assert!(!error.contains(encoded));
        }
    }
}

#[test]
fn rejects_untrusted_urls_and_keeps_remote_names_in_one_path_component() {
    let official = MediaClient::new(DEFAULT_CDN_BASE_URL).unwrap();
    for value in [
        "https://example.com/file?token=secret",
        "https://novac2c.cdn.weixin.qq.com.evil.test/file",
        "http://novac2c.cdn.weixin.qq.com/file",
        "https://novac2c.cdn.weixin.qq.com:444/file",
        "https://user:secret@novac2c.cdn.weixin.qq.com/file",
        "http://127.0.0.1:1234/file",
        "file:///etc/passwd",
    ] {
        let media = WeixinCdnMedia {
            full_url: value.into(),
            ..Default::default()
        };
        let error = official
            .download_url(Some(&media), "")
            .unwrap_err()
            .to_string();
        assert!(!error.contains("secret"));
    }
    assert!(MediaClient::new("https://example.com").is_err());
    let local = MediaClient::new("http://127.0.0.1:1234/cdn").unwrap();
    assert!(
        local
            .download_url(None, "http://127.0.0.1:1234/plain")
            .is_ok()
    );
    assert!(
        local
            .download_url(None, "http://127.0.0.1:1235/plain")
            .is_err()
    );
    assert_eq!(safe_filename("../../report.txt"), "report.txt");
    assert_eq!(
        safe_filename("C:\\Windows\\..\\report:ads.txt"),
        "report_ads.txt"
    );
    assert_eq!(safe_filename(".."), "file.bin");
    assert!(!safe_filename("nul\0.txt").contains('\0'));
    assert!(safe_filename(&"a".repeat(500)).len() <= 160);
}

#[tokio::test]
async fn downloads_encrypted_image_and_quoted_file_without_bot_headers() {
    let server = MockServer::start().await;
    let key = parse_hex_key(KEY).unwrap();
    Mock::given(method("GET"))
        .and(path("/cdn/download"))
        .and(query_param("encrypted_query_param", "image+/=&"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(encrypt(PNG, &key)))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/file"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(encrypt(b"report data", &key)))
        .expect(1)
        .mount(&server)
        .await;
    let message = message(json!([
        {
            "type": 2,
            "image_item": {
                "aeskey": KEY,
                "media": {"encrypt_query_param": "image+/=&", "aes_key": "bad-key"}
            }
        },
        {
            "type": 1,
            "text_item": {"text": "Please process these"},
            "ref_msg": {"message_item": {
                "type": 4,
                "file_item": {
                    "file_name": "../../report.txt",
                    "len": "11",
                    "md5": format!("{:x}", Md5::digest(b"report data")),
                    "media": {"full_url": format!("{}/file", server.uri()), "aes_key": STANDARD.encode(KEY)}
                }
            }}
        }
    ]));
    assert!(message.has_attachments());
    let root = tempfile::tempdir().unwrap();
    let dir = Inbox::create(root.path()).unwrap();
    let result = client(&server).download_attachments(&message, &dir).await;
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert_eq!(result.attachments.len(), 2);
    let image = &result.attachments[0];
    assert_eq!(image.kind, AttachmentKind::Image);
    assert_eq!(image.path.extension().unwrap(), "png");
    assert_eq!(std::fs::read(&image.path).unwrap(), PNG);
    let file = &result.attachments[1];
    assert_eq!(file.kind, AttachmentKind::File);
    assert!(
        file.path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with("-report.txt")
    );
    assert_eq!(
        file.path.parent().unwrap(),
        std::fs::canonicalize(dir.path()).unwrap()
    );
    assert_eq!(std::fs::read(&file.path).unwrap(), b"report data");
    for request in server.received_requests().await.unwrap() {
        for secret_header in ["authorization", "authorizationtype", "skroutetag", "cookie"] {
            assert!(!request.headers.contains_key(secret_header));
        }
    }
}

#[tokio::test]
async fn accepts_raw_key_and_plain_image_full_urls_without_following_redirects() {
    let server = MockServer::start().await;
    let key = parse_hex_key(KEY).unwrap();
    Mock::given(path("/raw"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(encrypt(PNG, &key)))
        .mount(&server)
        .await;
    Mock::given(path("/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(PNG))
        .mount(&server)
        .await;
    Mock::given(path("/redirect"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/never?key=secret", server.uri())),
        )
        .mount(&server)
        .await;
    Mock::given(path("/never"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let message = message(json!([
        {"type": 2, "image_item": {"media": {
            "full_url": format!("{}/raw", server.uri()), "aes_key": STANDARD.encode(key)
        }}},
        {"type": 2, "image_item": {"url": format!("{}/plain", server.uri())}},
        {"type": 2, "image_item": {"media": {"full_url": format!("{}/redirect?key=secret", server.uri())}}}
    ]));
    let root = tempfile::tempdir().unwrap();
    let dir = Inbox::create(root.path()).unwrap();
    let result = client(&server).download_attachments(&message, &dir).await;
    assert_eq!(result.attachments.len(), 2);
    assert_eq!(result.warnings.len(), 1);
    assert!(result.warnings[0].contains("302"));
    assert!(!result.warnings[0].contains("secret"));
    assert_ne!(result.attachments[0].path, result.attachments[1].path);
}

#[tokio::test]
async fn attachment_failures_leave_caption_and_other_attachments_available() {
    let server = MockServer::start().await;
    Mock::given(path("/denied"))
        .respond_with(ResponseTemplate::new(403).set_body_string("token=secret aes_key=secret"))
        .mount(&server)
        .await;
    let message = message(json!([
        {"type": 1, "text_item": {"text": "Keep this text"}},
        {"type": 2, "image_item": {"media": {"full_url": format!("{}/denied?key=secret", server.uri())}}},
        {"type": 4, "file_item": {"media": {"aes_key": "secret-invalid-key"}}},
        {"type": 2, "image_item": {"url": "https://evil.test/file?token=secret"}}
    ]));
    let root = tempfile::tempdir().unwrap();
    let dir = Inbox::create(root.path()).unwrap();
    let result = client(&server).download_attachments(&message, &dir).await;
    assert_eq!(message.text().as_deref(), Some("Keep this text"));
    assert!(result.attachments.is_empty());
    assert_eq!(result.warnings.len(), 3);
    assert!(
        result
            .warnings
            .iter()
            .all(|warning| !warning.contains("secret"))
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn rejects_truncated_media_metadata_mismatches_and_oversized_bodies() {
    let server = MockServer::start().await;
    Mock::given(path("/bad"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0; 15]))
        .mount(&server)
        .await;
    Mock::given(path("/wrong-size"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(encrypt(b"abc", &parse_hex_key(KEY).unwrap())),
        )
        .mount(&server)
        .await;
    Mock::given(path("/large"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0; 65]))
        .mount(&server)
        .await;
    let message = message(json!([
        {"type": 2, "image_item": {"media": {
            "full_url": format!("{}/bad", server.uri()), "aes_key": STANDARD.encode(KEY)
        }}},
        {"type": 4, "file_item": {"len": "100", "media": {
            "full_url": format!("{}/wrong-size", server.uri()), "aes_key": STANDARD.encode(KEY)
        }}}
    ]));
    let root = tempfile::tempdir().unwrap();
    let dir = Inbox::create(root.path()).unwrap();
    let result = client(&server).download_attachments(&message, &dir).await;
    assert!(result.attachments.is_empty());
    assert_eq!(result.warnings.len(), 2);
    let response = reqwest::get(format!("{}/large", server.uri()))
        .await
        .unwrap();
    assert!(read_bytes_limited(response, 64).await.is_err());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

#[tokio::test]
async fn quote_depth_and_attachment_count_are_bounded() {
    let mut nested = json!({"type": 2, "image_item": {}});
    for _ in 0..8 {
        nested =
            json!({"type": 1, "text_item": {"text": "quote"}, "ref_msg": {"message_item": nested}});
    }
    let deeply_quoted = message(json!([nested]));
    assert!(!deeply_quoted.has_attachments());
    assert!(deeply_quoted.text().unwrap().len() < 200);
    let root = tempfile::tempdir().unwrap();
    let dir = Inbox::create(root.path()).unwrap();
    let result = MediaClient::new(DEFAULT_CDN_BASE_URL)
        .unwrap()
        .download_attachments(&deeply_quoted, &dir)
        .await;
    assert_eq!(result.warnings.len(), 1);
    let many = message(serde_json::Value::Array(
        (0..20).map(|_| json!({"type": 2})).collect(),
    ));
    let (items, truncated) = attachment_items(&many);
    assert_eq!(items.len(), MAX_ATTACHMENTS);
    assert!(truncated);
}

#[tokio::test]
async fn downloads_encrypted_and_quoted_images_without_bot_headers() {
    let server = MockServer::start().await;
    let key = parse_hex_key(KEY).unwrap();
    Mock::given(method("GET"))
        .and(path("/cdn/download"))
        .and(query_param("encrypted_query_param", "image+/=&"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(encrypt(PNG, &key)))
        .expect(2)
        .mount(&server)
        .await;
    let image = json!({"type":2,"image_item":{"aeskey":KEY,
            "media":{"encrypt_query_param":"image+/=&","aes_key":"ignored"}}});
    let message = message(json!([image,
            {"type":1,"text_item":{"text":"caption"},"ref_msg":{"message_item":image}}]));
    let root = tempfile::tempdir().unwrap();
    let dir = Inbox::create(root.path()).unwrap();
    let result = client(&server).download_attachments(&message, &dir).await;
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert_eq!(result.attachments.len(), 2);
    assert_eq!(message.text().as_deref(), Some("caption"));
    for attachment in result.attachments {
        assert_eq!(attachment.kind, AttachmentKind::Image);
        assert_eq!(attachment.path.extension().unwrap(), "png");
        assert_eq!(std::fs::read(attachment.path).unwrap(), PNG);
    }
    for request in server.received_requests().await.unwrap() {
        for header in ["authorization", "authorizationtype", "skroutetag", "cookie"] {
            assert!(!request.headers.contains_key(header));
        }
    }
    server.verify().await;
}

#[tokio::test]
async fn rejects_truncated_images_and_oversized_response_bodies() {
    let server = MockServer::start().await;
    Mock::given(path("/bad"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0; 15]))
        .mount(&server)
        .await;
    Mock::given(path("/large"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0; 65]))
        .mount(&server)
        .await;
    let message = message(json!([{"type":2,"image_item":{"media":{
            "full_url":format!("{}/bad",server.uri()),"aes_key":STANDARD.encode(KEY)}}}]));
    let root = tempfile::tempdir().unwrap();
    let dir = Inbox::create(root.path()).unwrap();
    let result = client(&server).download_attachments(&message, &dir).await;
    assert!(result.attachments.is_empty());
    assert_eq!(result.warnings.len(), 1);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    let response = reqwest::get(format!("{}/large", server.uri()))
        .await
        .unwrap();
    assert!(read_bytes_limited(response, 64).await.is_err());
}

#[tokio::test]
async fn rejects_file_checksum_mismatch_before_saving() {
    let server = MockServer::start().await;
    Mock::given(path("/file"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(encrypt(b"report data", &parse_hex_key(KEY).unwrap())),
        )
        .mount(&server)
        .await;
    let message = message(json!([{"type":4,"file_item":{
        "file_name":"report.txt","len":"11","md5":"00000000000000000000000000000000",
        "media":{"full_url":format!("{}/file",server.uri()),"aes_key":STANDARD.encode(KEY)}
    }}]));
    let root = tempfile::tempdir().unwrap();
    let dir = Inbox::create(root.path()).unwrap();
    let result = client(&server).download_attachments(&message, &dir).await;
    assert!(message.has_attachments());
    assert!(result.attachments.is_empty());
    assert_eq!(result.warnings.len(), 1);
    assert!(result.warnings[0].contains("MD5"));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
}

fn encrypt(plaintext: &[u8], key: &[u8; 16]) -> Vec<u8> {
    ecb::Encryptor::<Aes128>::new(key.into()).encrypt_padded_vec_mut::<Pkcs7>(plaintext)
}

#[tokio::test]
async fn downloaded_files_are_owned_before_exposure_and_registration_failure_is_reported() {
    let server = MockServer::start().await;
    Mock::given(path("/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(PNG))
        .mount(&server)
        .await;
    let message = message(json!([{"type":2,"image_item":{
        "url":format!("{}/plain", server.uri())
    }}]));
    let root = tempfile::tempdir().unwrap();
    let inbox = Inbox::create(root.path()).unwrap();
    let result = client(&server).download_attachments(&message, &inbox).await;
    assert_eq!(result.attachments.len(), 1);
    assert!(result.warnings.is_empty());
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(inbox.path().join(".weixin-media.json")).unwrap())
            .unwrap();
    assert_eq!(marker["files"].as_array().unwrap().len(), 1);
    assert_eq!(
        marker["files"][0][0].as_str(),
        result.attachments[0].path.file_name().unwrap().to_str()
    );

    let blocked = Inbox::create(root.path()).unwrap();
    std::fs::rename(
        blocked.path().join(".weixin-media.json"),
        blocked.path().join("marker.backup"),
    )
    .unwrap();
    std::fs::create_dir(blocked.path().join(".weixin-media.json")).unwrap();
    let result = client(&server)
        .download_attachments(&message, &blocked)
        .await;
    assert!(result.attachments.is_empty());
    assert_eq!(result.warnings, ["无法登记微信附件归属"]);
    assert!(blocked.path().join("marker.backup").exists());
}
