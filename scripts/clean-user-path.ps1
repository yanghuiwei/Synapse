# Clean the *user* PATH (HKCU\Environment\Path):
#   * drop entries whose directory no longer exists
#   * drop duplicates (after expanding %VARS% and normalising the trailing backslash)
#
# Dry run by default -- nothing is written until you pass -Apply.
# A timestamped backup of the current value is always written to the Desktop first.
#
# Why not setx: setx silently truncates values at 1024 characters, which is the exact trap
# this script exists to avoid. We write the registry value directly and keep its ExpandString
# type, then broadcast WM_SETTINGCHANGE so already-open Explorer windows pick up the change.
#
# Context: the reqly installer refuses to touch a PATH of 1023+ characters because NSIS's
# ReadRegStr returns an EMPTY string (not a truncated one) at that size, so a naive
# read-modify-write would replace the whole PATH with a single directory.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts\clean-user-path.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\clean-user-path.ps1 -Apply

[CmdletBinding()]
param(
    [switch]$Apply
)

$ErrorActionPreference = 'Stop'

$keyPath   = 'HKCU:\Environment'
$valueName = 'Path'

if (-not (Test-Path $keyPath)) { throw "Registry key $keyPath not found." }

$rk   = Get-Item -Path $keyPath
$kind = $rk.GetValueKind($valueName).ToString()
$orig = (Get-ItemProperty -Path $keyPath -Name $valueName).Path

$stamp  = Get-Date -Format 'yyyyMMdd-HHmmss'
$backup = Join-Path ([Environment]::GetFolderPath('Desktop')) "user-path-backup-$stamp.txt"
[IO.File]::WriteAllText($backup, $orig, (New-Object Text.UTF8Encoding($false)))

Write-Host ''
Write-Host "Value kind      : $kind"
Write-Host "Original length : $($orig.Length) chars"
Write-Host "Backup          : $backup"
Write-Host ''

$seen    = @{}
$kept    = New-Object System.Collections.ArrayList
$dropped = New-Object System.Collections.ArrayList
$flagged = New-Object System.Collections.ArrayList

foreach ($raw in ($orig -split ';')) {
    $s = $raw.Trim()

    if ($s -eq '') {
        [void]$dropped.Add("blank entry          | $raw")
        continue
    }

    $expanded = [Environment]::ExpandEnvironmentVariables($s)
    $probe    = $expanded.TrimEnd('\')

    $norm = $probe.ToLowerInvariant()
    if ($seen.ContainsKey($norm)) {
        [void]$dropped.Add("duplicate            | $s")
        continue
    }

    # Two different questions, and they disagree exactly where it matters:
    #   Test-Path / Directory.Exists -> "is there a directory-ish name here?"
    #                                   (True for a junction whose target is gone -- checked, not assumed)
    #   enumerating it               -> "can anything actually be reached?"
    #                                   (False for that same junction)
    # A PATH entry only pays off if it is reachable, so the second question is the one that counts.
    $reachable = $false
    try {
        $null = [System.IO.Directory]::GetFileSystemEntries($probe)
        $reachable = $true
    } catch {
        $reachable = [System.IO.File]::Exists($probe)
    }
    $namePresent = Test-Path -LiteralPath $probe

    if (-not $reachable) {
        if ($namePresent) {
            # A reparse point whose target is gone. Reinstalling the owning app may well bring it
            # back, so this one is REPORTED, not removed -- we cannot tell "leftover junk" apart
            # from "temporarily broken" from here.
            [void]$flagged.Add("broken link          | $s")
        } else {
            [void]$dropped.Add("directory not found  | $s")
            continue
        }
    }

    $seen[$norm] = $true
    [void]$kept.Add($s)
}

$new = ($kept -join ';')

Write-Host "Kept    : $($kept.Count) entries"
Write-Host "Dropped : $($dropped.Count) entries"
foreach ($d in $dropped) { Write-Host "    - $d" }
if ($flagged.Count -gt 0) {
    Write-Host "Flagged : $($flagged.Count) entries (KEPT -- decide these yourself)"
    foreach ($f in $flagged) { Write-Host "    ! $f" }
}
Write-Host ''
Write-Host "New length : $($new.Length) chars   (installer limit: 1023)"
Write-Host ''

if ($new.Length -ge 1024) {
    Write-Host 'WARNING: the result is still 1024+ chars. Nothing was written.'
    Write-Host 'Remove some entries by hand before re-running.'
    exit 1
}

if ($new -eq $orig) {
    Write-Host 'Nothing to change.'
    exit 0
}

if (-not $Apply) {
    Write-Host 'DRY RUN -- nothing written. Re-run with -Apply to write the new value.'
    exit 0
}

$k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
try {
    $k.SetValue('Path', $new, [Microsoft.Win32.RegistryValueKind]::ExpandString)
} finally {
    $k.Close()
}

$check     = (Get-ItemProperty -Path $keyPath -Name $valueName).Path
$checkKind = (Get-Item -Path $keyPath).GetValueKind($valueName).ToString()

if ($check -ne $new) {
    Write-Host 'ERROR: read-back does not match what was written. Restore from the backup above.'
    exit 1
}

Write-Host "Written. Read-back verified: $($check.Length) chars, kind $checkKind"

try {
    $sig = '[DllImport("user32.dll", SetLastError=true, CharSet=CharSet.Auto)] public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);'
    $t   = Add-Type -MemberDefinition $sig -Name 'EnvBroadcast' -Namespace 'ReqlyTools' -PassThru
    $res = [UIntPtr]::Zero
    [void]$t::SendMessageTimeout([IntPtr]0xffff, 0x001A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$res)
    Write-Host 'Broadcast WM_SETTINGCHANGE.'
} catch {
    Write-Host "Broadcast skipped: $($_.Exception.Message)"
}

Write-Host ''
Write-Host 'Done. Open a NEW terminal for the change to take effect.'
