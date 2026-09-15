param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [string]$ReportDirectory = (Join-Path $PWD ("agentcheck-verify-" + [guid]::NewGuid().ToString()))
)
$ErrorActionPreference = 'Stop'
$source = (Resolve-Path $Binary).Path
$root = [IO.Path]::GetFullPath($ReportDirectory)
$work = Join-Path $root 'isolated'
New-Item -ItemType Directory -Force $work | Out-Null
Copy-Item $source (Join-Path $work 'agentcheck.exe')
$names = @('AGENTCHECK_CACHE_DIR', 'OMP_BIN', 'AGENTCHECK_GUI_PATH', 'PI_CODING_AGENT_DIR')
$previous = @{}
foreach ($name in $names) { $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
try {
    $env:AGENTCHECK_CACHE_DIR = Join-Path $root 'cache'
    $env:PI_CODING_AGENT_DIR = Join-Path $root 'empty-omp-config'
    Remove-Item Env:OMP_BIN -ErrorAction SilentlyContinue
    Remove-Item Env:AGENTCHECK_GUI_PATH -ErrorAction SilentlyContinue
    Push-Location $work
    try {
        & .\agentcheck.exe --version | Set-Content (Join-Path $root 'version.txt')
        if ($LASTEXITCODE -ne 0) { throw 'CLI 启动失败' }
        if (Test-Path $env:AGENTCHECK_CACHE_DIR) { throw '版本查询不应释放组件，请使用新的验证目录' }
        & .\agentcheck.exe --runtime-check | Set-Content (Join-Path $root 'runtime-first.json')
        if ($LASTEXITCODE -ne 0) { throw '首次组件运行检查失败' }
        & .\agentcheck.exe --runtime-check | Set-Content (Join-Path $root 'runtime-reused.json')
        if ($LASTEXITCODE -ne 0) { throw '重复启动检查失败' }
        if ($env:MODEL_URL -and $env:MODEL_ID -and $env:MODEL_API_KEY) {
            $reports = Join-Path $root 'model'
            & .\agentcheck.exe --url $env:MODEL_URL --model $env:MODEL_ID --no-gui --modules ingress --timeout-seconds 90 --report-dir $reports --html (Join-Path $reports 'report.html')
            if ($LASTEXITCODE -ne 0) { throw '真实模型检测流程失败' }
            $analysis = Get-Content -Raw (Join-Path $reports 'module-report/ingress.report.json') | ConvertFrom-Json
            if ($analysis.analyzer.status -notlike 'completed*') { throw '报告已生成，但分析程序没有完成判定，请检查报告' }
        }
    } finally { Pop-Location }
} finally {
    foreach ($name in $names) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
}
Write-Output "检查结果：$root"
