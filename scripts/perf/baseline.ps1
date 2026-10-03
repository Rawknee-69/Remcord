[CmdletBinding()]
param(
    [ValidateSet("Snapshot")]
    [string] $Mode = "Snapshot",
    [int] $RootProcessId,
    [ValidateRange(1, 60)]
    [int] $CpuSampleSeconds = 5,
    [string] $RunId = (Get-Date -Format "yyyyMMdd-HHmmss"),
    [string] $OutputRoot,
    [string] $UserDataPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
    $OutputRoot = Join-Path $PSScriptRoot "..\\..\\docs\\performance\\baseline\\runs"
}

function Get-ValueOrUnknown {
    param([object] $Value)
    if ($null -eq $Value -or [string]::IsNullOrWhiteSpace([string] $Value)) { return "UNKNOWN" }
    return $Value
}

function Get-DescendantProcessIds {
    param([int[]] $Roots, [object[]] $AllProcesses)
    $ids = [System.Collections.Generic.HashSet[int]]::new()
    $queue = [System.Collections.Generic.Queue[int]]::new()
    foreach ($root in $Roots) { if ($ids.Add($root)) { $queue.Enqueue($root) } }
    while ($queue.Count -gt 0) {
        $parent = $queue.Dequeue()
        foreach ($child in @($AllProcesses | Where-Object { [int] $_.ParentProcessId -eq $parent })) {
            if ($ids.Add([int] $child.ProcessId)) { $queue.Enqueue([int] $child.ProcessId) }
        }
    }
    return @($ids)
}

function Get-ProcessSample {
    param([int[]] $Ids, [object[]] $CimProcesses)
    $byId = @{}
    foreach ($process in $CimProcesses) { $byId[[int] $process.ProcessId] = $process }
    $rows = foreach ($id in $Ids) {
        $p = Get-Process -Id $id -ErrorAction SilentlyContinue
        $cim = $byId[$id]
        if ($null -eq $p -or $null -eq $cim) { continue }
        [ordered]@{
            PID = $id
            ParentPID = [int] $cim.ParentProcessId
            Name = $p.ProcessName
            CommandLine = Get-ValueOrUnknown $cim.CommandLine
            PrivateMemoryBytes = $p.PrivateMemorySize64
            WorkingSetBytes = $p.WorkingSet64
            CpuSeconds = Get-ValueOrUnknown $p.CPU
            Threads = $p.Threads.Count
            Handles = Get-ValueOrUnknown $p.HandleCount
        }
    }
    return @($rows)
}

function Get-GpuInfo {
    $gpus = @(Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue | ForEach-Object {
        [ordered]@{
            Name = Get-ValueOrUnknown $_.Name
            DriverVersion = Get-ValueOrUnknown $_.DriverVersion
            DriverDate = if ($_.DriverDate) { $_.DriverDate.ToString("o") } else { "UNKNOWN" }
            AdapterRAMBytes = Get-ValueOrUnknown $_.AdapterRAM
            CurrentResolution = if ($_.CurrentHorizontalResolution -and $_.CurrentVerticalResolution) { "$($_.CurrentHorizontalResolution)x$($_.CurrentVerticalResolution)" } else { "UNKNOWN" }
            CurrentRefreshHz = Get-ValueOrUnknown $_.CurrentRefreshRate
        }
    })
    if ($gpus.Count -eq 0) { return "UNKNOWN" }
    return $gpus
}

function Get-ConfigEvidence {
    param([string] $Path)
    if ([string]::IsNullOrWhiteSpace($Path)) {
        return [ordered]@{ UserDataPath = "UNKNOWN (pass -UserDataPath to inspect)"; Files = "UNKNOWN" }
    }
    if (-not (Test-Path -LiteralPath $Path -PathType Container)) {
        return [ordered]@{ UserDataPath = $Path; Files = "UNKNOWN (path does not exist)" }
    }
    $files = @("storage/settings.json", "storage/config.json", "quickCss.css", "shelter.js", "vencord.js", "vencord.css", "equicord.js", "equicord.css", "custom.js", "custom.css") |
        ForEach-Object {
            $candidate = Join-Path $Path $_
            [ordered]@{ RelativePath = $_; Exists = Test-Path -LiteralPath $candidate -PathType Leaf; Bytes = if (Test-Path -LiteralPath $candidate -PathType Leaf) { (Get-Item -LiteralPath $candidate).Length } else { "UNKNOWN" } }
        }
    $folders = @("themes", "plugins", "extensions") | ForEach-Object {
        $candidate = Join-Path $Path $_
        [ordered]@{ RelativePath = $_; Exists = Test-Path -LiteralPath $candidate -PathType Container; Entries = if (Test-Path -LiteralPath $candidate -PathType Container) { @(Get-ChildItem -LiteralPath $candidate -Force).Name } else { "UNKNOWN" } }
    }
    return [ordered]@{ UserDataPath = $Path; Files = $files; Folders = $folders; Note = "Presence and names only; no user-data file contents are copied." }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\\..")).Path
$package = Get-Content -Raw (Join-Path $repoRoot "package.json") | ConvertFrom-Json
$git = [ordered]@{
    Commit = (git -C $repoRoot rev-parse HEAD 2>$null)
    Branch = (git -C $repoRoot branch --show-current 2>$null)
    Dirty = [bool] (git -C $repoRoot status --porcelain 2>$null)
}
$computer = Get-CimInstance Win32_ComputerSystem
$os = Get-CimInstance Win32_OperatingSystem
$allCim = @(Get-CimInstance Win32_Process)

if ($RootProcessId -gt 0) {
    $rootIds = @($RootProcessId)
} else {
    # Match the product only by observed executable/command-line evidence; pass -RootProcessId if ambiguous.
    $rootIds = @($allCim | Where-Object { $_.Name -match "(?i)^legcord($|\\.exe$)" -or $_.CommandLine -match '(?i)(^|[\\/ ])legcord\.exe([ ]|$)' } | Select-Object -ExpandProperty ProcessId)
}
$rootIds = @($rootIds | Where-Object { $_ -in @($allCim.ProcessId) } | Select-Object -Unique)
$treeIds = @(if ($rootIds.Count -gt 0) { Get-DescendantProcessIds -Roots $rootIds -AllProcesses $allCim } else { @() })
$first = @(Get-ProcessSample -Ids $treeIds -CimProcesses $allCim)
Start-Sleep -Seconds $CpuSampleSeconds
$secondCim = @(Get-CimInstance Win32_Process)
$second = @(Get-ProcessSample -Ids $treeIds -CimProcesses $secondCim)
$firstByPid = @{}; foreach ($row in $first) { $firstByPid[$row.PID] = $row }
$cpuTotal = 0.0
foreach ($row in $second) {
    if ($firstByPid.ContainsKey($row.PID) -and $row.CpuSeconds -ne "UNKNOWN" -and $firstByPid[$row.PID].CpuSeconds -ne "UNKNOWN") {
        $cpu = (([double] $row.CpuSeconds - [double] $firstByPid[$row.PID].CpuSeconds) / $CpuSampleSeconds) * 100
        $row["CpuPercentOverSample"] = [math]::Round($cpu, 2)
        $cpuTotal += $cpu
    } else { $row["CpuPercentOverSample"] = "UNKNOWN" }
}
$totalWorkingSet = if ($second.Count) { ($second | Measure-Object -Property WorkingSetBytes -Sum).Sum } else { "UNKNOWN" }
$totalPrivateMemory = if ($second.Count) { ($second | Measure-Object -Property PrivateMemoryBytes -Sum).Sum } else { "UNKNOWN" }

$snapshot = [ordered]@{
    SchemaVersion = 1
    CapturedAt = (Get-Date).ToString("o")
    Mode = $Mode
    RunId = $RunId
    Methodology = "Read-only process-tree snapshot. CPU is elapsed process CPU divided by $CpuSampleSeconds seconds and is summed across processes; it may exceed 100% on multicore systems. This collector neither launches nor controls LEGcord."
    Build = [ordered]@{ Git = $git; LegcordVersion = $package.version; ElectronVersion = $package.devDependencies.electron; NodeVersion = (node --version); PackageManager = $package.packageManager }
    System = [ordered]@{ Windows = "$($os.Caption) $($os.Version) build $($os.BuildNumber)"; CPU = Get-ValueOrUnknown $computer.SystemType; CPUModel = Get-ValueOrUnknown (Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name); RAMBytes = $computer.TotalPhysicalMemory; GPU = Get-GpuInfo; VRR = "UNKNOWN (verify in Windows Advanced Display / GPU control panel)"; HardwareAcceleration = "UNKNOWN (verify current LEGcord configuration)" }
    ConfigurationEvidence = Get-ConfigEvidence -Path $UserDataPath
    ProcessTree = [ordered]@{ RootPIDs = if ($rootIds.Count) { $rootIds } else { "UNKNOWN (no automatic match; rerun with -RootProcessId)" }; ProcessCount = $second.Count; TotalWorkingSetBytes = $totalWorkingSet; TotalPrivateMemoryBytes = $totalPrivateMemory; TotalCpuPercentOverSample = if ($second.Count) { [math]::Round($cpuTotal, 2) } else { "UNKNOWN" }; Processes = $second }
    UnsupportedOrManualMetrics = @("Per-process GPU utilisation/memory", "frame-time percentiles and dropped frames", "long tasks/input delay", "startup milestones", "window visibility mode", "network and disk activity", "mod enabled state", "RPC enabled state")
}

$destination = Join-Path $OutputRoot "$RunId.json"
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
$snapshot | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $destination -Encoding utf8
Write-Host "Baseline snapshot written to $destination"
if ($rootIds.Count -eq 0) { Write-Warning "No LEGcord root process was identified. Start LEGcord normally and rerun with -RootProcessId <PID> for an unambiguous tree." }
