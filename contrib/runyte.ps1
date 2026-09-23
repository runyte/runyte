# SPDX-License-Identifier: MPL-2.0
# Dot-source this file from a Windows PowerShell 5.1 profile to enable :quit-here.
# PowerShell 7 uses the corresponding .NET ACL creation API.

function ConvertTo-RunyteNativeArguments {
    param([string[]] $Arguments)

    $runyteArgumentUnits = 0
    $runyteQuoted = foreach ($runyteArgument in $Arguments) {
        if ($null -eq $runyteArgument) { $runyteArgument = '' }
        if ($runyteArgument.IndexOf([char]0) -ge 0) { throw 'A Runyte argument contains NUL.' }
        $runyteArgumentUnits += $runyteArgument.Length + 3
        if ($runyteArgumentUnits -gt 32000) { throw 'Runyte arguments exceed the Windows command-line limit.' }
        $runyteBuilder = New-Object Text.StringBuilder
        [void] $runyteBuilder.Append('"')
        $runyteSlashes = 0
        foreach ($runyteCharacter in $runyteArgument.ToCharArray()) {
            if ($runyteCharacter -eq [char]92) {
                $runyteSlashes++
                continue
            }
            if ($runyteCharacter -eq [char]34) {
                [void] $runyteBuilder.Append([char]92, 2 * $runyteSlashes + 1)
            } else {
                [void] $runyteBuilder.Append([char]92, $runyteSlashes)
            }
            [void] $runyteBuilder.Append($runyteCharacter)
            $runyteSlashes = 0
        }
        [void] $runyteBuilder.Append([char]92, 2 * $runyteSlashes)
        [void] $runyteBuilder.Append('"')
        $runyteBuilder.ToString()
    }
    $runyteCommandLine = $runyteQuoted -join ' '
    if ($runyteCommandLine.Length -gt 32000) { throw 'Runyte arguments exceed the Windows command-line limit.' }
    return $runyteCommandLine
}

function Read-RunyteDirectoryHandoff {
    param([string] $Path)

    try {
        $runyteRecord = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    } catch [IO.FileNotFoundException] {
        return $null
    }
    try {
        if ($runyteRecord.Length -lt 14 -or $runyteRecord.Length -gt 65546) {
            throw 'Invalid Runyte directory handoff size.'
        }
        $runyteBytes = New-Object byte[] ([int] $runyteRecord.Length)
        $runyteOffset = 0
        while ($runyteOffset -lt $runyteBytes.Length) {
            $runyteRead = $runyteRecord.Read($runyteBytes, $runyteOffset, $runyteBytes.Length - $runyteOffset)
            if ($runyteRead -eq 0) { throw 'Truncated Runyte directory handoff.' }
            $runyteOffset += $runyteRead
        }
        if ($runyteRecord.ReadByte() -ne -1) { throw 'Runyte directory handoff changed while reading.' }
    } finally {
        $runyteRecord.Dispose()
    }
    $runyteMagic = [byte[]] (82, 78, 89, 67, 87, 68, 1, 0)
    for ($runyteIndex = 0; $runyteIndex -lt 8; $runyteIndex++) {
        if ($runyteBytes[$runyteIndex] -ne $runyteMagic[$runyteIndex]) {
            throw 'Unsupported Runyte directory handoff format.'
        }
    }
    $runyteCount = [BitConverter]::ToUInt32($runyteBytes, 8)
    if ($runyteCount -eq 0 -or $runyteCount -gt 32767 -or $runyteBytes.Length -ne 12 + 2 * $runyteCount) {
        throw 'Invalid Runyte directory handoff length.'
    }
    $runyteUnits = New-Object char[] ([int] $runyteCount)
    for ($runyteIndex = 0; $runyteIndex -lt $runyteCount; $runyteIndex++) {
        $runyteUnit = [int] $runyteBytes[12 + 2 * $runyteIndex] -bor ([int] $runyteBytes[13 + 2 * $runyteIndex] -shl 8)
        if ($runyteUnit -eq 0) { throw 'Runyte directory handoff contains NUL.' }
        $runyteUnits[$runyteIndex] = [char] $runyteUnit
    }
    # Construct directly from code units: no decoder may replace surrogates.
    return -join $runyteUnits
}

function runyte {
    $runyteArguments = @($args)
    $runyteDirectory = $null
    $runyteDirectoryOwned = $false
    $runyteTemporaryBase = $null
    $runyteRecordPath = $null
    $runyteProcess = $null
    $runyteExit = 1
    try {
        $runyteLocation = Get-Location
        if ($runyteLocation.Provider.Name -ne 'FileSystem') {
            throw 'Runyte requires a filesystem working directory.'
        }
        $runyteExecutable = (Get-Command runyte.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source
        $runyteTemporaryBase = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char]92)
        $runyteDirectory = [IO.Path]::Combine($runyteTemporaryBase + '\', ('runyte-cwd-' + [Guid]::NewGuid().ToString('N')))
        if ([IO.Directory]::Exists($runyteDirectory) -or [IO.File]::Exists($runyteDirectory)) {
            throw 'The Runyte handoff directory already exists.'
        }
        $runyteUser = [Security.Principal.WindowsIdentity]::GetCurrent()
        try {
            $runyteSecurity = New-Object Security.AccessControl.DirectorySecurity
            $runyteSecurity.SetOwner($runyteUser.User)
            $runyteSecurity.SetAccessRuleProtection($true, $false)
            $runyteRule = New-Object Security.AccessControl.FileSystemAccessRule(
                $runyteUser.User,
                [Security.AccessControl.FileSystemRights]::FullControl,
                [Security.AccessControl.InheritanceFlags]::None,
                [Security.AccessControl.PropagationFlags]::None,
                [Security.AccessControl.AccessControlType]::Allow
            )
            [void] $runyteSecurity.AddAccessRule($runyteRule)
            if ($PSVersionTable.PSEdition -eq 'Core') {
                [void] [IO.FileSystemAclExtensions]::CreateDirectory($runyteSecurity, $runyteDirectory)
            } else {
                [void] [IO.Directory]::CreateDirectory($runyteDirectory, $runyteSecurity)
            }
            $runyteDirectoryOwned = $true
        } finally {
            $runyteUser.Dispose()
        }
        $runyteRecordPath = [IO.Path]::Combine($runyteDirectory, 'cwd')
        $runyteStart = New-Object Diagnostics.ProcessStartInfo
        $runyteStart.FileName = $runyteExecutable
        $runyteStart.UseShellExecute = $false
        $runyteStart.WorkingDirectory = $runyteLocation.ProviderPath
        $runyteStart.Arguments = ConvertTo-RunyteNativeArguments (@('--cwd-file', $runyteRecordPath) + $runyteArguments)
        $runyteProcess = New-Object Diagnostics.Process
        $runyteProcess.StartInfo = $runyteStart
        if (-not $runyteProcess.Start()) { throw 'Runyte did not start.' }
        $runyteProcess.WaitForExit()
        $runyteExit = $runyteProcess.ExitCode
        if ($runyteExit -eq 0) {
            $runyteDestination = Read-RunyteDirectoryHandoff $runyteRecordPath
            if ($null -ne $runyteDestination) {
                if ($runyteDestination -notmatch '^(?:[A-Za-z]:\\|\\\\[^\\]+\\[^\\]+(?:\\|$))' -or
                    $runyteDestination.StartsWith('\\?\') -or $runyteDestination.StartsWith('\\.\')) {
                    throw 'Runyte did not return an ordinary absolute directory.'
                }
                Set-Location -LiteralPath $runyteDestination -ErrorAction Stop
            }
        }
    } catch {
        if ($runyteExit -eq 0) { $runyteExit = 1 }
        Write-Error -ErrorRecord $_
    } finally {
        $global:LASTEXITCODE = $runyteExit
        if ($null -ne $runyteProcess) { $runyteProcess.Dispose() }
        if ($runyteDirectoryOwned) {
            # Verify the exact fresh child and never recursively remove a tree.
            $runyteCleanupParent = [IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($runyteDirectory)).TrimEnd([char]92)
            $runyteCleanupName = [IO.Path]::GetFileName($runyteDirectory)
            if ([string]::Equals($runyteCleanupParent, $runyteTemporaryBase, [StringComparison]::OrdinalIgnoreCase) -and
                $runyteCleanupName -match '^runyte-cwd-[0-9a-f]{32}$') {
                if ($null -ne $runyteRecordPath) {
                    try { [IO.File]::Delete($runyteRecordPath) } catch { Write-Warning "Cannot remove Runyte handoff record: $_" }
                }
                try { [IO.Directory]::Delete($runyteDirectory, $false) } catch { Write-Warning "Cannot remove Runyte handoff directory: $_" }
            } else {
                Write-Warning 'Runyte handoff cleanup refused an unexpected path.'
            }
        }
    }
}
