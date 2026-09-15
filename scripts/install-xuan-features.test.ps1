$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$temporary = Join-Path (Join-Path $root 'target') ('xuan-installer-summary-test-' + [guid]::NewGuid().ToString('N'))
$failed = $false
try {
    $encoding = [System.Text.UTF8Encoding]::new($false, $true)
    $source = [System.IO.File]::ReadAllText((Join-Path $root 'install-xuan-features.bat'), $encoding)
    if ($source -match '(?<!\r)\n') { throw '批处理不能包含 LF-only 换行。' }
    foreach ($script in @('scripts\install-xuan-runtime.ps1', 'scripts\stop-xuan-plugin-processes.ps1')) {
        $bytes = [System.IO.File]::ReadAllBytes((Join-Path $root $script))
        if ($bytes.Length -lt 3 -or $bytes[0] -ne 239 -or $bytes[1] -ne 187 -or $bytes[2] -ne 191) {
            throw "Windows PowerShell 兼容脚本必须使用 UTF-8 BOM：$script"
        }
    }
    $pwshLookup = $source.IndexOf('where.exe pwsh.exe')
    $legacyFallback = $source.IndexOf('%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe')
    if ($pwshLookup -lt 0 -or $legacyFallback -lt 0 -or $pwshLookup -ge $legacyFallback) {
        throw '批处理必须优先使用 pwsh.exe，并回退到 Windows PowerShell。'
    }
    if ($source.IndexOf('"%POWERSHELL_CMD%" -NoLogo -NoProfile -NonInteractive %POWERSHELL_EXECUTION_POLICY% -File "%RUNTIME_INSTALLER%"') -lt 0) {
        throw '运行时安装必须使用已解析的 PowerShell 命令。'
    }
    if ($source.IndexOf('if not defined POWERSHELL_CMD set "POWERSHELL_EXECUTION_POLICY=-ExecutionPolicy Bypass"') -lt 0) {
        throw 'Windows PowerShell 回退必须绕过当前进程的执行策略。'
    }
    if ($source.IndexOf('dir /b /s "%LOCALAPPDATA%\OpenAI\Codex\bin\rg.exe"') -lt 0 -or
        $source.IndexOf('for %%D in ("%RIPGREP_CMD%") do set "PATH=%%~dpD;%PATH%"') -lt 0) {
        throw '批处理必须能定位 Codex 内置 rg.exe，并供后续子进程使用。'
    }
    $start = $source.IndexOf('echo [7/7]')
    $end = $source.IndexOf(':require_command', $start)
    if ($start -lt 0 -or $end -lt 0) { throw '未找到安装结束提示。' }
    $tail = $source.Substring($start, $end - $start)
    if ($tail -notmatch '^echo \[7/7\][^\r\n]+\r\n\r\necho {3}\[通过\]') {
        throw '完成提示的中文输出之间必须保留空行。'
    }
    [System.IO.Directory]::CreateDirectory($temporary) | Out-Null
    $fixture = Join-Path $temporary 'summary.cmd'
    $wrapper = Join-Path $temporary 'wrapper.cmd'
    [System.IO.File]::WriteAllText($fixture, "@echo off`r`nchcp 65001 >nul`r`n$tail", $encoding)
    [System.IO.File]::WriteAllText($wrapper, "@echo off`r`nchcp 65001 >nul`r`ncall `"%~dp0summary.cmd`"`r`nexit /b %errorlevel%`r`n", $encoding)
    $startInfo = [System.Diagnostics.ProcessStartInfo]::new($env:ComSpec)
    $startInfo.ArgumentList.Add('/d')
    $startInfo.ArgumentList.Add('/c')
    $startInfo.ArgumentList.Add($wrapper)
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.StandardOutputEncoding = $encoding
    $startInfo.StandardErrorEncoding = $encoding
    $child = [System.Diagnostics.Process]::Start($startInfo)
    try {
        $stdout = $child.StandardOutput.ReadToEndAsync()
        $stderr = $child.StandardError.ReadToEndAsync()
        if (-not $child.WaitForExit(10000)) {
            $child.Kill()
            $child.WaitForExit()
            throw '安装完成提示测试超时。'
        }
        $output = $stdout.GetAwaiter().GetResult()
        $errors = $stderr.GetAwaiter().GetResult()
        if ($child.ExitCode -ne 0 -or $errors) { throw "安装完成提示执行失败：$errors" }
        if ($output -notmatch '插件自行管理通信和子进程，不修改 Codex\+\+ 程序或更新逻辑。') {
            throw '安装完成提示未完整输出。'
        }
        [pscustomobject]@{ utf8WithoutBom = $true; crlf = $true; completionOutput = $true } | ConvertTo-Json -Compress
    } finally {
        $child.Dispose()
    }
} catch {
    $failed = $true
    Write-Error $_ -ErrorAction Continue
} finally {
    $expectedParent = [System.IO.Path]::GetFullPath((Join-Path $root 'target')) + [System.IO.Path]::DirectorySeparatorChar
    $resolved = [System.IO.Path]::GetFullPath($temporary)
    if (-not $resolved.StartsWith($expectedParent, [StringComparison]::OrdinalIgnoreCase)) { throw '测试清理目标越界。' }
    if (Test-Path -LiteralPath $resolved) {
        foreach ($file in Get-ChildItem -LiteralPath $resolved -File -Force) {
            Remove-Item -LiteralPath $file.FullName -Force
        }
        [System.IO.Directory]::Delete($resolved, $false)
    }
}
if ($failed) { exit 1 }
