#[test]
fn standalone_rice_maid_pet_contract() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new("node")
        .arg(root.join("tools/rice-maid-pet/tests/contract.mjs"))
        .current_dir(&root)
        .output()
        .expect("node is required");
    assert!(
        output.status.success(),
        "rice maid pet contract failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
