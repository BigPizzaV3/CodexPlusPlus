use sha2::{Digest, Sha256};

fn assert_sha256(relative_path: &str, expected: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative_path);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "failed to read upstream theme asset {}: {error}",
            path.display()
        )
    });
    // Git may materialize these text assets with CRLF on Windows. Hash the
    // repository's canonical LF representation so the guard is platform
    // independent while still detecting substantive asset changes.
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\r' && bytes.get(index + 1) == Some(&b'\n') {
            index += 1;
        }
        normalized.push(bytes[index]);
        index += 1;
    }
    let actual = format!("{:X}", Sha256::digest(normalized));
    assert_eq!(actual, expected, "upstream asset changed: {relative_path}");
}

#[test]
fn bundled_target_renderers_and_styles_remain_byte_exact() {
    for (path, hash) in [
        (
            "assets/inject/upstream/dream-skin/windows/renderer-inject.js",
            "18E5E99719042A3D877915F70C0D023B6914C8458EF004B40F5FF8E9FE6D5C3F",
        ),
        (
            "assets/inject/upstream/dream-skin/windows/dream-skin.css",
            "049695F3F8FD66826F7DD0EF9363D21A5AA491C627DD9602CEAEA7383CFDD49C",
        ),
        (
            "assets/inject/upstream/dream-skin/macos/renderer-inject.js",
            "73DA118C964E768676C44C9ABAC910114547DDA44B6190CC3D8A6220059ABB0B",
        ),
        (
            "assets/inject/upstream/dream-skin/macos/dream-skin.css",
            "CDA12A5E08815533919A6005A803C2269637CDCAAC4D121D170230163DC9CF09",
        ),
        (
            "assets/inject/upstream/cidala-tiger/windows/renderer-inject.js",
            "98B9F323A9711FFB3E8307C9ACA3F4FBDA71896A047F334AB38AF43FDB2D53E6",
        ),
        (
            "assets/inject/upstream/cidala-tiger/windows/dream-skin.css",
            "0C371B7D794C4783648D1733661E8FA8674C872296CE5CF9898B28EB1765425C",
        ),
        (
            "assets/inject/upstream/cidala-tiger/macos/renderer-inject.js",
            "19202C8A37C7512E65F950A5516A314867FDF305B74B313F0ABCEA8CF7347F59",
        ),
        (
            "assets/inject/upstream/cidala-tiger/macos/dream-skin.css",
            "45506CA7C71D4E9867287AE2358C4380C0993F0D04039C29FEE6DBEE20495148",
        ),
        (
            "assets/inject/upstream/snow-skin/renderer-inject.js",
            "9AE8123B51917975B5D4B91995173A6A4DD3C27C6BD5B465B5670C2C1330955A",
        ),
        (
            "assets/inject/upstream/snow-skin/dream-skin.css",
            "97807DE20E40680471D211466B657867CB46280F393EF9D7FBBA5CE829AE5599",
        ),
        (
            "assets/inject/upstream/glass-vision/renderer-inject.js",
            "D14943E95DB62DB81BF29D9CF14FCAF1DD1EA9A9625245C020865127EEA295A2",
        ),
        (
            "assets/inject/upstream/glass-vision/glass-vision.css",
            "4C37C53544EE4F1CD93BA5D0DC3E174B05D4CB84EC9A436295D11D19F0BB04F1",
        ),
    ] {
        assert_sha256(path, hash);
    }
}

#[test]
fn bundled_skin_pack_theme_files_remain_byte_exact() {
    for (path, hash) in [
        (
            "caishen-lite",
            "379CB601522E7A5C2FC906E3D8BD5C7C64385FBD2A428798F8D169DEB7026F2E",
        ),
        (
            "caishen-max",
            "0CFF815EC9582B88ECF4AD9E7562D7DFF32904FA8F09338811397960D62FD7D4",
        ),
        (
            "caishen-readable",
            "EB1FFD4F2F49137B4AEDDBED435513D42685C8ED9E97DF644C693FD7859CC62D",
        ),
        (
            "export-night",
            "C312329AABEE84B9A8443B08D4DB64863EC49DEEA3C7F7C942B57E4391B87B59",
        ),
        (
            "global-founder-bright",
            "EAFB018494225ABABD83AE0E7B940E3F565232CF8F30C53AAFA63E7652178810",
        ),
        (
            "mythic-guardian-noir",
            "2A57716D0161F7405D713912BCD0CD329038657518537F3EFDB5F7EE53DBAE3D",
        ),
    ] {
        assert_sha256(
            &format!("assets/inject/upstream/skin-packs/packs/{path}/theme.json"),
            hash,
        );
    }
}
