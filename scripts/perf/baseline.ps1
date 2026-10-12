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
    foreach ($id in @($Ids | Select-Object -Unique)) {
        $cim = $byId[$id]
        $p = $null
        $failure = $null
        try { $p = Get-Process -Id $id -ErrorAction Stop } catch { $failure = $_.Exception.Message }
        if ($null -eq $p) {
            # Keep the row so an exit or access failure is visible rather than silently omitted.
            [pscustomobject]@{
                PID = $id
                ParentPID = if ($null -ne $cim) { [int] $cim.ParentProcessId } else { "UNKNOWN" }
                Name = if ($null -ne $cim) { Get-ValueOrUnknown $cim.Name } else { "UNKNOWN" }
                CommandLine = if ($null -ne $cim) { Get-ValueOrUnknown $cim.CommandLine } else { "UNKNOWN" }
                PrivateMemoryBytes = "UNKNOWN"
                WorkingSetBytes = "UNKNOWN"
                CpuSeconds = "UNKNOWN"
                Threads = "UNKNOWN"
                Handles = "UNKNOWN"
                MeasurementStatus = if ($null -ne $cim) { "PROCESS_DISAPPEARED_OR_ACCESS_DENIED" } else { "PROCESS_NOT_PRESENT" }
                MeasurementError = Get-ValueOrUnknown $failure
            }
            continue
        }
        $privateMemory = $workingSet = $cpu = $threads = $handles = "UNKNOWN"
        $propertyFailure = $null
        try { $privateMemory = Get-ValueOrUnknown $p.PrivateMemorySize64 } catch { $propertyFailure = $_.Exception.Message }
        try { $workingSet = Get-ValueOrUnknown $p.WorkingSet64 } catch { $propertyFailure = $_.Exception.Message }
        try { $cpu = Get-ValueOrUnknown $p.CPU } catch { $propertyFailure = $_.Exception.Message }
        try { $threads = Get-ValueOrUnknown $p.Threads.Count } catch { $propertyFailure = $_.Exception.Message }
        try { $handles = Get-ValueOrUnknown $p.HandleCount } catch { $propertyFailure = $_.Exception.Message }
        [pscustomobject]@{
            PID = $id
            ParentPID = if ($null -ne $cim) { [int] $cim.ParentProcessId } else { "UNKNOWN" }
            Name = $p.ProcessName
            CommandLine = if ($null -ne $cim) { Get-ValueOrUnknown $cim.CommandLine } else { "UNKNOWN" }
            PrivateMemoryBytes = $privateMemory
            WorkingSetBytes = $workingSet
            CpuSeconds = $cpu
            Threads = $threads
            Handles = $handles
            MeasurementStatus = if ($null -ne $cim) { "PRESENT" } else { "CIM_DATA_UNAVAILABLE" }
            MeasurementError = Get-ValueOrUnknown $propertyFailure
        }
    }
}

function Get-NumericTotalOrUnknown {
    param([object[]] $Rows, [string] $Property)
    $values = @($Rows | ForEach-Object {
        $propertyInfo = @($_.PSObject.Properties.Match($Property) | Select-Object -First 1)
        if ($propertyInfo.Count -eq 0) { return }
        $value = $propertyInfo[0].Value
        if ($value -is [byte] -or $value -is [int16] -or $value -is [int32] -or $value -is [int64] -or $value -is [uint16] -or $value -is [uint32] -or $value -is [uint64] -or $value -is [double] -or $value -is [decimal]) { [double] $value }
    })
    if ($values.Count -eq 0) { return "UNKNOWN" }
    return [int64] (($values | Measure-Object -Sum).Sum)
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
        [ordered]@{ RelativePath = $_; Exists = Test-Path -LiteralPath $candidate -PathType Container; Entries = if (Test-Path -LiteralPath $candidate -PathType Container) { @(Get-ChildItem -LiteralPath $candidate -Force | Select-Object -ExpandProperty Name) } else { "UNKNOWN" } }
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
    $rootIds = @($allCim | Where-Object { $_.Name -match "(?i)^remcord($|\\.exe$)" -or $_.CommandLine -match '(?i)(^|[\\/ ])remcord\.exe([ ]|$)' } | Select-Object -ExpandProperty ProcessId)
}
$rootIds = @($rootIds | Where-Object { $_ -in @($allCim.ProcessId) } | Select-Object -Unique)
$treeIds = @(if ($rootIds.Count -gt 0) { Get-DescendantProcessIds -Roots $rootIds -AllProcesses $allCim } else { @() })
$first = @(Get-ProcessSample -Ids $treeIds -CimProcesses $allCim)
Start-Sleep -Seconds $CpuSampleSeconds
$secondCim = @(Get-CimInstance Win32_Process)
$secondTreeIds = @(if ($rootIds.Count -gt 0) { Get-DescendantProcessIds -Roots $rootIds -AllProcesses $secondCim } else { @() })
# Union both trees: exits remain visible and children created during the CPU interval are included.
$trackedIds = @($treeIds + $secondTreeIds | Select-Object -Unique)
$second = @(Get-ProcessSample -Ids $trackedIds -CimProcesses $secondCim)
$firstByPid = @{}; foreach ($row in $first) { $firstByPid[$row.PID] = $row }
$cpuTotal = 0.0
foreach ($row in $second) {
    if ($row.MeasurementStatus -eq "PRESENT" -and $firstByPid.ContainsKey($row.PID) -and $row.CpuSeconds -ne "UNKNOWN" -and $firstByPid[$row.PID].CpuSeconds -ne "UNKNOWN") {
        $cpu = (([double] $row.CpuSeconds - [double] $firstByPid[$row.PID].CpuSeconds) / $CpuSampleSeconds) * 100
        $row | Add-Member -NotePropertyName "CpuPercentOverSample" -NotePropertyValue ([math]::Round($cpu, 2)) -Force
        $cpuTotal += $cpu
    } else { $row | Add-Member -NotePropertyName "CpuPercentOverSample" -NotePropertyValue "UNKNOWN" -Force }
}
$totalWorkingSet = Get-NumericTotalOrUnknown -Rows $second -Property "WorkingSetBytes"
$totalPrivateMemory = Get-NumericTotalOrUnknown -Rows $second -Property "PrivateMemoryBytes"
$observedProcessCount = @($second | Where-Object { $_.MeasurementStatus -eq "PRESENT" -or $_.MeasurementStatus -eq "CIM_DATA_UNAVAILABLE" }).Count
$rootProcessStatus = @($rootIds | ForEach-Object {
    $rootId = $_
    $row = @($second | Where-Object { $_.PID -eq $rootId } | Select-Object -First 1)
    if ($row.Count -eq 0) { "ROOT_NOT_TRACKED" } else { $row[0].MeasurementStatus }
})

$snapshot = [ordered]@{
    SchemaVersion = 1
    CapturedAt = (Get-Date).ToString("o")
    Mode = $Mode
    RunId = $RunId
    Methodology = "Read-only process-tree snapshot. CPU is elapsed process CPU divided by $CpuSampleSeconds seconds and is summed across processes; it may exceed 100% on multicore systems. This collector neither launches nor controls REMCORD."
    Build = [ordered]@{ Git = $git; RemcordVersion = $package.version; ElectronVersion = $package.devDependencies.electron; NodeVersion = (node --version); PackageManager = $package.packageManager }
    System = [ordered]@{ Windows = "$($os.Caption) $($os.Version) build $($os.BuildNumber)"; CPU = Get-ValueOrUnknown $computer.SystemType; CPUModel = Get-ValueOrUnknown (Get-CimInstance Win32_Processor | Select-Object -First 1 -ExpandProperty Name); RAMBytes = $computer.TotalPhysicalMemory; GPU = Get-GpuInfo; VRR = "UNKNOWN (verify in Windows Advanced Display / GPU control panel)"; HardwareAcceleration = "UNKNOWN (verify current REMCORD configuration)" }
    ConfigurationEvidence = Get-ConfigEvidence -Path $UserDataPath
    ProcessTree = [ordered]@{ RootPIDs = if ($rootIds.Count) { $rootIds } else { "UNKNOWN (no automatic match; rerun with -RootProcessId)" }; RootProcessStatus = if ($rootProcessStatus.Count) { $rootProcessStatus } else { "UNKNOWN" }; ProcessCount = $observedProcessCount; TotalWorkingSetBytes = $totalWorkingSet; TotalPrivateMemoryBytes = $totalPrivateMemory; TotalCpuPercentOverSample = if ($observedProcessCount -gt 0) { [math]::Round($cpuTotal, 2) } else { "UNKNOWN" }; Processes = $second }
    UnsupportedOrManualMetrics = @("Per-process GPU utilisation/memory", "frame-time percentiles and dropped frames", "long tasks/input delay", "startup milestones", "window visibility mode", "network and disk activity", "mod enabled state", "RPC enabled state")
}

$destination = Join-Path $OutputRoot "$RunId.json"
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $destination) | Out-Null
$snapshot | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $destination -Encoding utf8
Write-Host "Baseline snapshot written to $destination"
if ($rootIds.Count -eq 0) { Write-Warning "No REMCORD root process was identified. Start REMCORD normally and rerun with -RootProcessId <PID> for an unambiguous tree." }
