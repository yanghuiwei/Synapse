# Regression tests for src-tauri/nsis/path-hook.nsh -- the NSIS hook that adds the install
# directory to the current user's PATH on install, and removes it on uninstall.
#
# Why this file exists: touching HKCU\Environment\Path is the one place in the installer where a
# bug silently damages the user's machine instead of failing loudly. NSIS 3.11 behaviour,
# MEASURED (see the P1/P2/P3 cases below), not assumed:
#
#   value <= 1023 chars  ->  ReadRegStr returns it in full
#   value >= 1024 chars  ->  ReadRegStr FAILS and returns "" (empty -- not a truncated string!)
#   StrCpy               ->  caps its result at 1023 chars, silently truncating the rest
#
# So a naive read-modify-write on a long PATH reads "" and then writes "C:\...\reqly" as the
# WHOLE user PATH, wiping every other entry without a word. That is what these tests pin down.
#
# The tests point the same macros at a scratch key (HKCU\Software\ReqlyPathHookSelfTest) and run
# the real compiled code, so the real PATH is never touched.
#
# Usage (Windows PowerShell):
#     powershell -ExecutionPolicy Bypass -File scripts\nsis-path-hook-test.ps1
#
# Exit code 0 = all cases passed, 1 = something failed.

$ErrorActionPreference = 'Stop'

$repoRoot = Split-Path -Parent $PSScriptRoot
$nsisDir  = Join-Path $repoRoot 'src-tauri\nsis'
$makensis = Join-Path $env:LOCALAPPDATA 'tauri\NSIS\makensis.exe'
$selftest = Join-Path $nsisDir 'path-hook.selftest.exe'
$key      = 'HKCU:\Software\ReqlyPathHookSelfTest'

$script:results = New-Object System.Collections.ArrayList
$script:failCount = 0
$script:inst = ''

function Add-Result {
    param([string]$Case, [string]$Label, [string]$Expected, [string]$Actual)
    if ($Expected -ceq $Actual) {
        [void]$script:results.Add([pscustomobject]@{ Case = $Case; Label = $Label; Result = 'PASS' })
    } else {
        $script:failCount++
        [void]$script:results.Add([pscustomobject]@{
            Case = $Case; Label = $Label; Result = 'FAIL'
            Detail = "expected <$Expected> got <$Actual>"
        })
    }
}

function Add-Failure {
    param([string]$Case, [string]$Detail)
    $script:failCount++
    [void]$script:results.Add([pscustomobject]@{ Case = $Case; Label = ''; Result = 'FAIL'; Detail = $Detail })
}

# Shorten the install dir to a marker so the report stays readable.
function Get-Short {
    param([AllowNull()][string]$Value)
    if ($null -eq $Value) { return '' }
    if ([string]::IsNullOrEmpty($script:inst)) { return $Value }
    return ($Value -replace [regex]::Escape($script:inst), '<INST>')
}

# NOTE: "-Absent" is not the same as "-Value ''". Windows and our hook treat "no Path value"
# and "a Path value that is empty" as different situations, so the tests must too.
function Set-PathRaw {
    param([string]$Value = '', [string]$PropertyType = 'ExpandString', [switch]$Absent)
    Remove-ItemProperty -Path $key -Name 'Path' -ErrorAction SilentlyContinue
    if (-not $Absent) {
        New-ItemProperty -Path $key -Name 'Path' -Value $Value -PropertyType $PropertyType -Force | Out-Null
    }
}

function Get-PathRaw {
    $item = Get-ItemProperty -Path $key -Name 'Path' -ErrorAction SilentlyContinue
    if ($null -eq $item) { return $null }
    return $item.Path
}

function Get-PathRawKind {
    $rk = Get-Item -Path $key
    return $rk.GetValueKind('Path').ToString()
}

function Invoke-Hook {
    param([string]$Mode)
    $p = Start-Process -FilePath $selftest -ArgumentList '/S', "/MODE=$Mode" -Wait -PassThru
    if ($p.ExitCode -ne 0) { throw "path-hook.selftest.exe /MODE=$Mode exited with $($p.ExitCode)" }
}

# Repeated 'C:\aaa;' segments plus filler, so no entry ever equals the install dir.
function New-LenString {
    param([int]$Length)
    $unit = 'C:\aaa;'
    $s = ''
    while ($s.Length + $unit.Length -le $Length) { $s += $unit }
    if ($s.Length -lt $Length) { $s += ('z' * ($Length - $s.Length)) }
    return $s
}

# Shortest form we expect the hook to produce from $Orig (it strips trailing ';' before joining).
function Get-ExpectedAppend {
    param([string]$Orig, [string]$InstDir)
    return ($Orig.TrimEnd(';') + ';' + $InstDir)
}

# ---------------------------------------------------------------- preflight

if (-not (Test-Path $makensis)) { throw "makensis not found at $makensis" }

# makensis reads the .nsh/.nsi as ACP(GBK) when there is no BOM, mangling the Chinese comments
# and reporting "Bad text encoding" -- which looks like a syntax error. Fail early and loud.
foreach ($src in @('path-hook.nsh', 'path-hook.selftest.nsi')) {
    $full = Join-Path $nsisDir $src
    $bytes = [System.IO.File]::ReadAllBytes($full)
    if (-not ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF)) {
        throw "$src is not UTF-8 with BOM. Fix with:`n  `$p='$full'; `$t=[IO.File]::ReadAllText(`$p,[Text.UTF8Encoding]::new(`$false)); [IO.File]::WriteAllText(`$p,`$t,[Text.UTF8Encoding]::new(`$true))"
    }
}

Write-Host '== building path-hook.selftest.exe =='
& $makensis /V2 (Join-Path $nsisDir 'path-hook.selftest.nsi') | Out-Null
if (-not (Test-Path $selftest)) { throw 'makensis produced no exe' }

if (Test-Path $key) { Remove-Item $key -Recurse -Force }
New-Item -Path $key -Force | Out-Null

# ---------------------------------------------------------------- P: measured NSIS limits

try {
    Set-PathRaw -Absent
    New-ItemProperty -Path $key -Name 'Probe' -Value ('P' * 1023) -PropertyType ExpandString -Force | Out-Null
    Invoke-Hook 'probe'
    $props = Get-ItemProperty -Path $key
    Add-Result 'P1' 'read of a 1023-char value is complete' '1023' "$($props.ProbeReadLen)"
    Add-Result 'P2' 'write cap is 1023 chars' '1023' "$($props.ProbeWriteLen)"

    New-ItemProperty -Path $key -Name 'Probe' -Value ('P' * 1024) -PropertyType ExpandString -Force | Out-Null
    Invoke-Hook 'probe'
    # The trap: one char over the limit and the value reads back as EMPTY, not truncated.
    Add-Result 'P3' 'read of a 1024-char value FAILS into an empty string' '0' "$((Get-ItemProperty -Path $key).ProbeReadLen)"
} catch {
    Add-Failure 'P' $_.Exception.Message
}

# Install dir as the hook itself sees it -- not assumed, read from the hook.
try {
    Set-PathRaw 'C:\probe'
    Invoke-Hook 'append'
    $script:inst = (Get-ItemProperty -Path $key).LastInstDir
    if ([string]::IsNullOrEmpty($script:inst)) { throw 'hook reported an empty $INSTDIR' }
} catch {
    Add-Failure 'preflight' $_.Exception.Message
}
$inst = $script:inst

$readMax  = 1023
$writeMax = 1023

# ---------------------------------------------------------------- append cases

function Test-Case {
    param([string]$Name, [string]$Initial = '', [string]$Expected = '', [string]$Mode = 'append', [string]$PropertyType = 'ExpandString', [switch]$Absent)
    try {
        Set-PathRaw $Initial $PropertyType -Absent:$Absent
        Invoke-Hook $Mode
        Add-Result $Name $Mode (Get-Short $Expected) (Get-Short (Get-PathRaw))
    } catch {
        Add-Failure $Name $_.Exception.Message
    }
}

# C1: no Path value at all -> it becomes just our directory
Test-Case 'C1' '' $inst -Absent

# C2: normal append goes to the end
Test-Case 'C2' 'C:\aaa;C:\bbb' "C:\aaa;C:\bbb;$inst"

# C3: idempotent -- a second install must not add a duplicate
Test-Case 'C3' "C:\aaa;$inst" "C:\aaa;$inst"

# C4: Windows compares PATH entries case-insensitively
Test-Case 'C4' $inst.ToUpper() $inst.ToUpper()

# C5: trailing backslash is the same directory
Test-Case 'C5' "$inst\" "$inst\"

# C6: quoted entry is the same directory
Test-Case 'C6' "`"$inst`"" "`"$inst`""

# C7: no doubled separator when the old value ends with ';'
Test-Case 'C7' 'C:\aaa;' "C:\aaa;$inst"

# C8: a common prefix is NOT a match (C:\reqly vs C:\reqly2)
Test-Case 'C8' "${inst}2" "${inst}2;$inst"

# C9: a Path value that EXISTS but is empty is deliberately left alone. We cannot tell it apart
# from "exists and is too long to read" using core NSIS instructions, and guessing wrong there is
# the catastrophic case, so the hook refuses to touch it. Cost is one missing PATH entry.
Test-Case 'C9' '' ''

# C10: THE catastrophic case -- a >=1024 char PATH reads back as "" and must be left untouched
$longPath = 'C:\' + ('x' * 1200) + ';C:\aaa'
Test-Case 'C10' $longPath $longPath

# C11: same guard on the uninstall side
Test-Case 'C11' $longPath $longPath 'remove'

# ---------------------------------------------------------------- length boundary

try {
    $atLimit = New-LenString ($writeMax - $inst.Length - 1)
    Set-PathRaw $atLimit
    Invoke-Hook 'append'
    $expected = Get-ExpectedAppend $atLimit $inst
    Add-Result "C12 ($($atLimit.Length)+1+$($inst.Length)=$($expected.Length) chars)" 'append while it still fits' (Get-Short $expected) (Get-Short (Get-PathRaw))

    $oneOver = New-LenString ($writeMax - $inst.Length)
    Set-PathRaw $oneOver
    Invoke-Hook 'append'
    Add-Result "C13 ($($oneOver.Length)+1+$($inst.Length)=$($writeMax + 1) chars)" 'append would overflow -> nothing written' (Get-Short $oneOver) (Get-Short (Get-PathRaw))
} catch {
    Add-Failure 'C12/C13' $_.Exception.Message
}

# ---------------------------------------------------------------- remove cases

Test-Case 'C14' "C:\aaa;$inst;C:\bbb" 'C:\aaa;C:\bbb' 'remove'
Test-Case 'C15' "C:\aaa;$inst" 'C:\aaa' 'remove'
Test-Case 'C16' $inst '' 'remove'
Test-Case 'C17' 'C:\aaa;C:\bbb' 'C:\aaa;C:\bbb' 'remove'
Test-Case 'C18' "C:\aaa;$($inst.ToUpper())" 'C:\aaa' 'remove'
Test-Case 'C19' "C:\aaa;$inst\" 'C:\aaa' 'remove'

# ---------------------------------------------------------------- value kind + report

try {
    Set-PathRaw 'C:\aaa'
    Invoke-Hook 'append'
    Add-Result 'C20' 'written value keeps the expandable type' 'ExpandString' (Get-PathRawKind)
} catch {
    Add-Failure 'C20' $_.Exception.Message
}

Remove-ItemProperty -Path $key -Name Probe -ErrorAction SilentlyContinue
Remove-Item -Path $key -Recurse -Force -ErrorAction SilentlyContinue

Write-Host ''
foreach ($r in $script:results) {
    $line = "{0,-6} {1,-7} {2,-52} {3}" -f $r.Case, $r.Result, $r.Label, $r.Detail
    Write-Host $line.TrimEnd()
}
$passed = ($script:results | Where-Object { $_.Result -eq 'PASS' }).Count
Write-Host ''
Write-Host "== $passed passed / $($script:failCount) failed =="
if ($script:failCount -gt 0) { exit 1 }
exit 0
