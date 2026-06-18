$settingsFile = $args[0]
$c = Get-Content $settingsFile -Raw

$pattern = 'pub model_mappings: HashMap<String, String>,'
$replacement = "pub model_mappings: HashMap<String, String>,`r`n    #[serde(rename = ""modelMappingsEnabled"", default = ""default_true"")]`r`n    pub model_mappings_enabled: bool,"
$c = $c -replace [regex]::Escape($pattern), $replacement

$pattern2 = 'model_mappings: HashMap::new(),'
$replacement2 = "model_mappings: HashMap::new(),`r`n            model_mappings_enabled: true,"
$c = $c -replace [regex]::Escape($pattern2), $replacement2

if ($c -notmatch 'fn default_true\(\)') {
    $c = $c -replace 'fn default_none\(\)', "fn default_true() -> bool { true }`r`n`r`nfn default_none()"
}

[System.IO.File]::WriteAllText($settingsFile, $c, [System.Text.UTF8Encoding]::new($false))
Write-Output "[OK] settings.rs updated"