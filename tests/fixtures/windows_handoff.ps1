# SPDX-License-Identifier: MPL-2.0
param([ValidateSet('Helpers', 'Editor', 'Failure', 'ReadFailure', 'LocationFailure')] [string] $Mode)
$ErrorActionPreference = 'Stop'
. $env:RUNYTE_HANDOFF_WRAPPER

function Assert-Handoff([bool] $Condition, [string] $Message) {
    if (-not $Condition) { throw $Message }
}

if ($Mode -eq 'Helpers') {
    $runyteWords = @(
        '--exact', 'handoff_argument_fixture', '--ignored', '--nocapture', '--test-threads=1', '--',
        '', 'two words', ('unicode ' + [char]0x00e9 + [char]0xd83d + [char]0xde00),
        'one"two', 'trailing\', 'quoted\"tail\\', '$literal; & [brackets] `ticks', '-leading'
    )
    $runyteStart = New-Object Diagnostics.ProcessStartInfo
    $runyteStart.FileName = $env:RUNYTE_HANDOFF_NATIVE_FIXTURE
    $runyteStart.UseShellExecute = $false
    $runyteStart.Arguments = ConvertTo-RunyteNativeArguments $runyteWords
    $runyteChild = [Diagnostics.Process]::Start($runyteStart)
    try {
        if (-not $runyteChild.WaitForExit(10000)) {
            $runyteChild.Kill()
            throw 'Argument fixture timed out.'
        }
        Assert-Handoff ($runyteChild.ExitCode -eq 0) 'Argument fixture failed.'
    } finally {
        $runyteChild.Dispose()
    }
    $runyteReceived = [IO.File]::ReadAllText([IO.Path]::Combine($env:RUNYTE_HANDOFF_ROOT, 'arguments.json')) | ConvertFrom-Json
    Assert-Handoff ($runyteReceived.Count -eq $runyteWords.Count) 'Argument count changed.'
    for ($runyteIndex = 0; $runyteIndex -lt $runyteWords.Count; $runyteIndex++) {
        Assert-Handoff ([string]::Equals($runyteReceived[$runyteIndex], $runyteWords[$runyteIndex], [StringComparison]::Ordinal)) "Argument $runyteIndex changed."
    }
    foreach ($runyteBadArguments in @(@(('nul' + [char]0)), @(('x' * 32001)))) {
        $runyteRejected = $false
        try { [void] (ConvertTo-RunyteNativeArguments $runyteBadArguments) } catch { $runyteRejected = $true }
        Assert-Handoff $runyteRejected 'Invalid command line was accepted.'
    }

    $runytePath = [IO.Path]::Combine($env:RUNYTE_HANDOFF_ROOT, 'record')
    Assert-Handoff ($null -eq (Read-RunyteDirectoryHandoff $runytePath)) 'Missing record was not empty.'
    # Data-only fixture: encode lone surrogates manually, never through a
    # replacement-fallback encoder. Decoding must preserve those units.
    $runyteUnits = [UInt16[]] (67, 58, 92, 0xd800, 32, 0xdc00)
    $runyteRecord = New-Object byte[] (12 + 2 * $runyteUnits.Length)
    [byte[]] (82, 78, 89, 67, 87, 68, 1, 0) | ForEach-Object -Begin { $runyteIndex = 0 } -Process { $runyteRecord[$runyteIndex++] = $_ }
    [BitConverter]::GetBytes([UInt32] $runyteUnits.Length).CopyTo($runyteRecord, 8)
    for ($runyteIndex = 0; $runyteIndex -lt $runyteUnits.Length; $runyteIndex++) {
        [BitConverter]::GetBytes($runyteUnits[$runyteIndex]).CopyTo($runyteRecord, 12 + 2 * $runyteIndex)
    }
    [IO.File]::WriteAllBytes($runytePath, $runyteRecord)
    $runyteDecoded = Read-RunyteDirectoryHandoff $runytePath
    Assert-Handoff ($runyteDecoded.Length -eq $runyteUnits.Length) 'Decoded length changed.'
    for ($runyteIndex = 0; $runyteIndex -lt $runyteUnits.Length; $runyteIndex++) {
        Assert-Handoff ([int] $runyteDecoded[$runyteIndex] -eq $runyteUnits[$runyteIndex]) 'UTF-16 unit changed.'
    }
    foreach ($runyteCase in @('magic', 'count', 'nul', 'truncated', 'trailing', 'oversized')) {
        $runyteInvalid = [byte[]] $runyteRecord.Clone()
        switch ($runyteCase) {
            'magic' { $runyteInvalid[6] = 2 }
            'count' { $runyteInvalid[8] = 0 }
            'nul' { $runyteInvalid[12] = 0; $runyteInvalid[13] = 0 }
            'truncated' { $runyteInvalid = [byte[]] $runyteInvalid[0..($runyteInvalid.Length - 2)] }
            'trailing' { $runyteInvalid = [byte[]] ($runyteInvalid + [byte]1) }
            'oversized' { $runyteInvalid = New-Object byte[] 65547 }
        }
        [IO.File]::WriteAllBytes($runytePath, $runyteInvalid)
        $runyteRejected = $false
        try { [void] (Read-RunyteDirectoryHandoff $runytePath) } catch { $runyteRejected = $true }
        Assert-Handoff $runyteRejected "Malformed $runyteCase record was accepted."
    }
    exit 0
}

Set-Location -LiteralPath ([IO.Path]::Combine($env:RUNYTE_HANDOFF_ROOT, 'project'))
if ($Mode -eq 'ReadFailure' -or $Mode -eq 'LocationFailure') {
    # A successful native command must not mask a failed handoff. Inject only
    # the record boundary; exercise the real wrapper's catch/status/cleanup.
    if ($Mode -eq 'ReadFailure') {
        function Read-RunyteDirectoryHandoff {
            param([string] $Path)
            throw 'Injected malformed handoff record.'
        }
    } else {
        function Read-RunyteDirectoryHandoff {
            param([string] $Path)
            return [IO.Path]::Combine($env:RUNYTE_HANDOFF_ROOT, 'missing-destination')
        }
    }
    $runytePreviousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        runyte --version
    } finally {
        $ErrorActionPreference = $runytePreviousPreference
    }
} elseif ($Mode -eq 'Failure') {
    # The wrapper must preserve a real nonzero native exit without ending the
    # PowerShell caller. A missing explicitly named config fails before editing.
    runyte --standalone --config ([IO.Path]::Combine($env:RUNYTE_HANDOFF_ROOT, 'missing.yaml')) -- $env:RUNYTE_HANDOFF_TARGET
} else {
    runyte --standalone --config $env:RUNYTE_HANDOFF_CONFIG --project-root (Get-Location).ProviderPath -- $env:RUNYTE_HANDOFF_TARGET
}
$runyteResult = @{
    code = $global:LASTEXITCODE
    directory = (Get-Location).ProviderPath
    remaining = @([IO.Directory]::GetFileSystemEntries($env:TEMP)).Count
} | ConvertTo-Json -Compress
[IO.File]::WriteAllText([IO.Path]::Combine($env:RUNYTE_HANDOFF_ROOT, 'result.json'), $runyteResult)
Write-Output 'POWERSHELL_CALLER_CONTINUED'
