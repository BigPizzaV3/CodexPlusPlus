$proxyFile = $args[0]
$c = Get-Content $proxyFile -Raw

# Change request_json to mut request_json
$oldSig = "fn upstream_request_parts(" + [char]10 + "    relay: &crate::settings::RelayProfile," + [char]10 + "    request_json: Value," + [char]10 + ")"
$newSig = "fn upstream_request_parts(" + [char]10 + "    relay: &crate::settings::RelayProfile," + [char]10 + "    mut request_json: Value," + [char]10 + ")"
$c = $c -replace [regex]::Escape($oldSig), $newSig

# Add model rewriting block before match relay.protocol
$matchBlock = "-> anyhow::Result<(String, Value, UpstreamWireApi)> {" + [char]10 + "    match relay.protocol {"
$rewriteBlock = "-> anyhow::Result<(String, Value, UpstreamWireApi)> {" + [char]10 +
    "    // === Model name rewriting ===" + [char]10 +
    "    if relay.model_mappings_enabled && !relay.model_mappings.is_empty() {" + [char]10 +
    "        if let Some(model_val) = request_json.get(""model"") {" + [char]10 +
    "            let model_str = model_val.as_str().unwrap_or_default();" + [char]10 +
    "            if !model_str.is_empty() {" + [char]10 +
    "                if let Some(mapped_to) = relay.model_mappings.get(model_str) {" + [char]10 +
    "                    let _ = crate::diagnostic_log::append_diagnostic_log(" + [char]10 +
    '                        "protocol_proxy.model_rewrite",' + [char]10 +
    "                        serde_json::json!({""from"": model_str, ""to"": mapped_to})," + [char]10 +
    "                    );" + [char]10 +
    '                    request_json["model"] = serde_json::json!(mapped_to);' + [char]10 +
    "                }" + [char]10 +
    "            }" + [char]10 +
    "        }" + [char]10 +
    "    }" + [char]10 +
    "    match relay.protocol {"
$c = $c -replace [regex]::Escape($matchBlock), $rewriteBlock

[System.IO.File]::WriteAllText($proxyFile, $c, [System.Text.UTF8Encoding]::new($false))
Write-Output "[OK] protocol_proxy.rs updated"