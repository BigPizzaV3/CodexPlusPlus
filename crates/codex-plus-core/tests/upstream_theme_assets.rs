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
    let actual = format!("{:X}", Sha256::digest(bytes));
    assert_eq!(actual, expected, "upstream asset changed: {relative_path}");
}

#[test]
fn bundled_target_renderers_and_styles_remain_byte_exact() {
    for (path, hash) in [
        (
            "assets/inject/upstream/dream-skin/windows/renderer-inject.js",
            "D9383160DB0A71CE8219A845D3C300AFDDC874932B533FE3550DA723125E38A5",
        ),
        (
            "assets/inject/upstream/dream-skin/windows/dream-skin.css",
            "99DD5DA043DCE71766E4C01BECA22A123284BA5DCD72D75AA9D36B4549B7D542",
        ),
        (
            "assets/inject/upstream/dream-skin/macos/renderer-inject.js",
            "2704C39506C66554C3529BF0D15B876B4AEC2DD9A36B1796AB43E19C33A046FC",
        ),
        (
            "assets/inject/upstream/dream-skin/macos/dream-skin.css",
            "EC3C3BC5F6E10E20A3F2307796BD1E1350E80E5D23D37318EE5468833C95A6DF",
        ),
        (
            "assets/inject/upstream/cidala-tiger/windows/renderer-inject.js",
            "97C1F062F6695C19469E851390974121F29C66B690C4790E761C0E1F82586EF1",
        ),
        (
            "assets/inject/upstream/cidala-tiger/windows/dream-skin.css",
            "82ECECF50F3595B80BD148D55246FA2871E3F3D0A2C9031F5BAD8B5E6413E666",
        ),
        (
            "assets/inject/upstream/cidala-tiger/macos/renderer-inject.js",
            "21FAF1DC0A3EBE78D8D972182CACE62BD93D5D0E5841725398A4A524EF2BC20B",
        ),
        (
            "assets/inject/upstream/cidala-tiger/macos/dream-skin.css",
            "5E149E9A13985961C5F3125296178ACB2ABF0B528974F1E616AA625970430562",
        ),
        (
            "assets/inject/upstream/snow-skin/renderer-inject.js",
            "0FCDFF4AECD03EAB2CA4EE923CCD20CB97EB5460F7C9F07351A2003FFA76E6FA",
        ),
        (
            "assets/inject/upstream/snow-skin/dream-skin.css",
            "0AF2D20FBE3E3DD13F0BE7F1E5A90366E1501084827B22C1D4815A421BFCE823",
        ),
        (
            "assets/inject/upstream/glass-vision/renderer-inject.js",
            "57A529C0F5743CC7068B5F9064AAB098137520A051E5B0C5A45AD2DFAB91E98C",
        ),
        (
            "assets/inject/upstream/glass-vision/glass-vision.css",
            "84D4AF19D9D5B7D5934139892F83CDB58B5EB370598D775A54587C285A2C8BC1",
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
            "68F6AA3C9C68D18014D51E7076A71D9B3F5CA156F339CE3F001F394F0217F941",
        ),
        (
            "caishen-max",
            "02D886D75F779E30E05EB6D6CABC68A9A07EB94B9BCCB4561B82A511DE14F31D",
        ),
        (
            "caishen-readable",
            "5E9947AF7AA00A5CC871330AD55CD9694E49AF54F265C622293C386519F570CB",
        ),
        (
            "export-night",
            "CB07ADE8952BC809497F78F2E73CC886F43F57E4866FC265B7D4E788D63AEE74",
        ),
        (
            "global-founder-bright",
            "6ED25E22A5D9229AD7C7DED3B71EC818D70B0DAE45A650401B8F902F8FA367B9",
        ),
        (
            "mythic-guardian-noir",
            "F4D30003D0F2346C49CECD6398072DB1FEA78ADB8335844DA6ADABE0DDEBA417",
        ),
    ] {
        assert_sha256(
            &format!("assets/inject/upstream/skin-packs/packs/{path}/theme.json"),
            hash,
        );
    }
}
