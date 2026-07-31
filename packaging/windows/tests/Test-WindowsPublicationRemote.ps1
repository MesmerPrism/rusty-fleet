# Copyright (C) 2026 Rusty Fleet contributors
# SPDX-License-Identifier: AGPL-3.0-or-later

[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Assert-RemoteTest([bool] $Condition, [string] $Message) {
    if (-not $Condition) {
        throw $Message
    }
}

function Write-RemoteTestJson(
    [string] $LiteralPath,
    [object] $InputObject
) {
    [IO.File]::WriteAllText(
        $LiteralPath,
        (ConvertTo-Json -InputObject $InputObject -Depth 30 -Compress),
        [Text.UTF8Encoding]::new($false)
    )
}

function New-RemoteTestState {
    [ordered]@{
        source_revision = $script:sourceRevision
        alternate_revision = $script:alternateRevision
        ref_type = "commit"
        ref_sha = $script:sourceRevision
        tag_objects = @()
        tag_check_count = 0
        move_on_tag_check = 0
        fail_tag = $false
        malformed_tag = $false
        fail_release_list = $false
        malformed_release_list = $false
        create_fail_after_state = $false
        create_failure_consumed = $false
        upload_fail_after = 0
        upload_failure_consumed = $false
        fail_visibility = $false
        malformed_visibility = $false
        fail_latest = $false
        latest_tag = "v1.2.2"
        next_release_id = 7001
        releases = @()
    }
}

function New-RemoteAsset([object] $Asset) {
    [pscustomobject][ordered]@{
        name = $Asset.name
        size = [long] $Asset.size_bytes
        digest = "sha256:$($Asset.sha256)"
        state = "uploaded"
    }
}

function New-RemoteRelease {
    param(
        [Parameter(Mandatory)]
        [AllowEmptyCollection()]
        [object[]] $Assets,
        [bool] $Draft = $true,
        [bool] $Prerelease = $true
    )

    [pscustomobject][ordered]@{
        id = 7001
        tag_name = "v1.2.3"
        draft = $Draft
        prerelease = $Prerelease
        assets = @($Assets)
    }
}

function Invoke-RemotePublication {
    param(
        [Parameter(Mandatory)][string] $StatePath,
        [Parameter(Mandatory)][object[]] $Assets
    )

    $priorStatePath = [Environment]::GetEnvironmentVariable(
        "RUSTY_FLEET_FAKE_GH_STATE",
        [EnvironmentVariableTarget]::Process
    )
    try {
        [Environment]::SetEnvironmentVariable(
            "RUSTY_FLEET_FAKE_GH_STATE",
            $StatePath,
            [EnvironmentVariableTarget]::Process
        )
        $guardState = [pscustomobject]@{
            count = 0
        }
        $guard = {
            $guardState.count++
        }.GetNewClosure()
        $result = Publish-RustyFleetGitHubRelease `
            -GitHubRepository "MesmerPrism/rusty-fleet" `
            -Tag "v1.2.3" `
            -ExpectedSourceRevision $script:sourceRevision `
            -AssetInventory $Assets `
            -Prerelease $true `
            -GhExecutable $script:fakeGhPath `
            -AssertLocalState $guard
        Assert-RemoteTest `
            -Condition ($guardState.count -ge 4) `
            -Message "remote authority omitted its local-state guard"
        return $result
    }
    finally {
        [Environment]::SetEnvironmentVariable(
            "RUSTY_FLEET_FAKE_GH_STATE",
            $priorStatePath,
            [EnvironmentVariableTarget]::Process
        )
    }
}

function Assert-RemoteRejected {
    param(
        [Parameter(Mandatory)][scriptblock] $Action,
        [Parameter(Mandatory)][string] $Context
    )

    $rejected = $false
    try {
        & $Action | Out-Null
    }
    catch {
        $rejected = $true
    }
    Assert-RemoteTest $rejected "$Context was not rejected"
}

$modulePath = Join-Path $PSScriptRoot "..\Publication.Remote.psm1"
Import-Module $modulePath -Force
$testRoot = Join-Path ([IO.Path]::GetTempPath()) (
    "rusty-fleet-remote-publication-test-$([Guid]::NewGuid().ToString('N'))"
)
[IO.Directory]::CreateDirectory($testRoot) | Out-Null
$script:sourceRevision = "1111111111111111111111111111111111111111"
$script:alternateRevision = "2222222222222222222222222222222222222222"
$script:fakeGhPath = Join-Path $testRoot "fake-gh.ps1"
$priorFakeState = [Environment]::GetEnvironmentVariable(
    "RUSTY_FLEET_FAKE_GH_STATE",
    [EnvironmentVariableTarget]::Process
)

try {
    $fakeGhText = @'
param([Parameter(ValueFromRemainingArguments = $true)][string[]] $GhArgs)

$ErrorActionPreference = "Stop"
$statePath = [Environment]::GetEnvironmentVariable(
    "RUSTY_FLEET_FAKE_GH_STATE",
    [EnvironmentVariableTarget]::Process
)
if (-not $statePath) {
    exit 91
}
$state = Get-Content -LiteralPath $statePath -Raw |
    ConvertFrom-Json -Depth 30

function Save-State {
    [IO.File]::WriteAllText(
        $statePath,
        (ConvertTo-Json -InputObject $state -Depth 30 -Compress),
        [Text.UTF8Encoding]::new($false)
    )
}

function Write-Json([object] $Value) {
    [Console]::Out.Write(
        (ConvertTo-Json -InputObject $Value -Depth 30 -Compress)
    )
}

function Get-Endpoint {
    foreach ($argument in $GhArgs) {
        if ($argument -like "repos/*") {
            return $argument
        }
    }
    return ""
}

if ($GhArgs.Count -lt 1) {
    exit 92
}
if ($GhArgs[0] -ceq "api") {
    $endpoint = Get-Endpoint
    if ($endpoint -like "repos/*/git/ref/tags/*") {
        if ($state.fail_tag) {
            exit 41
        }
        if ($state.malformed_tag) {
            [Console]::Out.Write("{malformed")
            exit 0
        }
        $state.tag_check_count = [int] $state.tag_check_count + 1
        $targetType = [string] $state.ref_type
        $targetSha = [string] $state.ref_sha
        if ([int] $state.move_on_tag_check -gt 0 -and
            [int] $state.tag_check_count -ge
                [int] $state.move_on_tag_check) {
            $targetType = "commit"
            $targetSha = [string] $state.alternate_revision
        }
        Save-State
        Write-Json ([ordered]@{
            ref = "refs/tags/v1.2.3"
            object = [ordered]@{
                type = $targetType
                sha = $targetSha
            }
        })
        exit 0
    }
    if ($endpoint -like "repos/*/git/tags/*") {
        $sha = Split-Path -Leaf $endpoint
        $match = @(
            $state.tag_objects |
                Where-Object { $_.sha -ceq $sha }
        )
        if ($match.Count -ne 1) {
            exit 42
        }
        Write-Json ([ordered]@{
            sha = [string] $match[0].sha
            tag = [string] $match[0].tag
            object = [ordered]@{
                type = [string] $match[0].target_type
                sha = [string] $match[0].target_sha
            }
        })
        exit 0
    }
    if ($endpoint -ceq "repos/MesmerPrism/rusty-fleet/releases") {
        if ($state.fail_release_list) {
            exit 43
        }
        if ($state.malformed_release_list) {
            [Console]::Out.Write('{"not":"an-array"}')
            exit 0
        }
        Write-Json @($state.releases)
        exit 0
    }
    if ($endpoint -ceq "repos/MesmerPrism/rusty-fleet/releases/latest") {
        if ($state.fail_latest) {
            exit 52
        }
        Write-Json ([ordered]@{
            tag_name = [string] $state.latest_tag
        })
        exit 0
    }
    if ($endpoint -like "repos/*/releases/*" -and
        $GhArgs -contains "PATCH") {
        if ($state.fail_visibility) {
            exit 50
        }
        if ($state.malformed_visibility) {
            [Console]::Out.Write("{malformed")
            exit 0
        }
        $releaseId = [long] (Split-Path -Leaf $endpoint)
        $matches = @(
            $state.releases |
                Where-Object { [long] $_.id -eq $releaseId }
        )
        if ($matches.Count -ne 1) {
            exit 44
        }
        $matches[0].draft = $false
        Save-State
        Write-Json $matches[0]
        exit 0
    }
    exit 45
}

if ($GhArgs.Count -ge 3 -and
    $GhArgs[0] -ceq "release" -and
    $GhArgs[1] -ceq "create") {
    if (@($state.releases).Count -ne 0) {
        exit 46
    }
    $release = [pscustomobject][ordered]@{
        id = [long] $state.next_release_id
        tag_name = [string] $GhArgs[2]
        draft = $true
        prerelease = [bool] ($GhArgs -contains "--prerelease")
        assets = @()
    }
    $state.releases = @($release)
    $mustFail = (
        [bool] $state.create_fail_after_state -and
        -not [bool] $state.create_failure_consumed
    )
    if ($mustFail) {
        $state.create_failure_consumed = $true
    }
    Save-State
    if ($mustFail) {
        exit 51
    }
    exit 0
}

if ($GhArgs.Count -ge 4 -and
    $GhArgs[0] -ceq "release" -and
    $GhArgs[1] -ceq "upload") {
    $paths = [Collections.Generic.List[string]]::new()
    for ($index = 3; $index -lt $GhArgs.Count; $index++) {
        if ($GhArgs[$index] -ceq "--repo") {
            break
        }
        $paths.Add($GhArgs[$index])
    }
    $release = @(
        $state.releases |
            Where-Object { $_.tag_name -ceq $GhArgs[2] }
    )
    if ($release.Count -ne 1 -or -not $release[0].draft) {
        exit 47
    }
    $limit = $paths.Count
    $mustFail = (
        [int] $state.upload_fail_after -gt 0 -and
        -not [bool] $state.upload_failure_consumed
    )
    if ($mustFail) {
        $limit = [Math]::Min([int] $state.upload_fail_after, $paths.Count)
    }
    for ($index = 0; $index -lt $limit; $index++) {
        $path = $paths[$index]
        $info = Get-Item -LiteralPath $path
        $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $path).
            Hash.ToLowerInvariant()
        $asset = [pscustomobject][ordered]@{
            name = $info.Name
            size = [long] $info.Length
            digest = "sha256:$hash"
            state = "uploaded"
        }
        $release[0].assets = @($release[0].assets) + $asset
    }
    if ($mustFail) {
        $state.upload_failure_consumed = $true
    }
    Save-State
    if ($mustFail) {
        exit 48
    }
    exit 0
}
exit 49
'@
    [IO.File]::WriteAllText(
        $script:fakeGhPath,
        $fakeGhText,
        [Text.UTF8Encoding]::new($false)
    )

    $assetRoot = Join-Path $testRoot "assets"
    [IO.Directory]::CreateDirectory($assetRoot) | Out-Null
    $assetInventory = @(
        "alpha.bin",
        "beta.json",
        "gamma.zip"
    ) | ForEach-Object {
        $path = Join-Path $assetRoot $_
        [IO.File]::WriteAllText(
            $path,
            "remote-publication-fixture-$_",
            [Text.UTF8Encoding]::new($false)
        )
        [pscustomobject][ordered]@{
            name = $_
            sha256 = (
                Get-FileHash -Algorithm SHA256 -LiteralPath $path
            ).Hash.ToLowerInvariant()
            size_bytes = (Get-Item -LiteralPath $path).Length
            path = (Resolve-Path -LiteralPath $path).Path
        }
    }
    $exactRemoteAssets = @($assetInventory | ForEach-Object {
        New-RemoteAsset $_
    })

    $statePath = Join-Path $testRoot "state.json"
    Write-RemoteTestJson $statePath (New-RemoteTestState)
    $created = Invoke-RemotePublication `
        -StatePath $statePath `
        -Assets $assetInventory
    $createdState = Get-Content -LiteralPath $statePath -Raw |
        ConvertFrom-Json -Depth 30
    Assert-RemoteTest (
        $created.draft_verified -and
        $created.visible_verified -and
        $created.remote_tag_verified -and
        $created.remote_integrity_verified -and
        -not $created.resumed_draft -and
        $created.uploaded_asset_count -eq $assetInventory.Count -and
        @($createdState.releases).Count -eq 1 -and
        $createdState.releases[0].draft -eq $false -and
        @($createdState.releases[0].assets).Count -eq $assetInventory.Count
    ) "create, upload, verify, and visibility flow did not close exactly"

    $latestAlphaState = New-RemoteTestState
    $latestAlphaState.releases = @(
        New-RemoteRelease -Assets $exactRemoteAssets
    )
    $latestAlphaState.latest_tag = "v1.2.3"
    Write-RemoteTestJson $statePath $latestAlphaState
    Assert-RemoteRejected -Context "alpha latest release" -Action {
        Invoke-RemotePublication -StatePath $statePath -Assets $assetInventory
    }

    $latestLookupFailure = New-RemoteTestState
    $latestLookupFailure.releases = @(
        New-RemoteRelease -Assets $exactRemoteAssets
    )
    $latestLookupFailure.fail_latest = $true
    Write-RemoteTestJson $statePath $latestLookupFailure
    Assert-RemoteRejected -Context "latest release lookup failure" -Action {
        Invoke-RemotePublication -StatePath $statePath -Assets $assetInventory
    }

    $partialState = New-RemoteTestState
    $partialState.upload_fail_after = 1
    Write-RemoteTestJson $statePath $partialState
    Assert-RemoteRejected -Context "partial upload" -Action {
        Invoke-RemotePublication `
            -StatePath $statePath `
            -Assets $assetInventory
    }
    $afterPartial = Get-Content -LiteralPath $statePath -Raw |
        ConvertFrom-Json -Depth 30
    Assert-RemoteTest (
        @($afterPartial.releases).Count -eq 1 -and
        $afterPartial.releases[0].draft -eq $true -and
        @($afterPartial.releases[0].assets).Count -eq 1
    ) "partial upload did not leave an exact resumable draft"
    $resumed = Invoke-RemotePublication `
        -StatePath $statePath `
        -Assets $assetInventory
    Assert-RemoteTest (
        $resumed.resumed_draft -and
        $resumed.uploaded_asset_count -eq ($assetInventory.Count - 1)
    ) "partial draft did not resume by uploading only missing assets"

    $createFailureState = New-RemoteTestState
    $createFailureState.create_fail_after_state = $true
    Write-RemoteTestJson $statePath $createFailureState
    Assert-RemoteRejected -Context "partial draft creation response" -Action {
        Invoke-RemotePublication `
            -StatePath $statePath `
            -Assets $assetInventory
    }
    $afterCreateFailure = Get-Content -LiteralPath $statePath -Raw |
        ConvertFrom-Json -Depth 30
    Assert-RemoteTest (
        @($afterCreateFailure.releases).Count -eq 1 -and
        $afterCreateFailure.releases[0].draft -eq $true -and
        @($afterCreateFailure.releases[0].assets).Count -eq 0
    ) "partial create failure did not retain an empty resumable draft"
    $createResumed = Invoke-RemotePublication `
        -StatePath $statePath `
        -Assets $assetInventory
    Assert-RemoteTest (
        $createResumed.resumed_draft -and
        $createResumed.uploaded_asset_count -eq $assetInventory.Count
    ) "empty draft from a partial create failure did not resume"

    $completeState = New-RemoteTestState
    $completeState.releases = @(
        New-RemoteRelease -Assets $exactRemoteAssets
    )
    Write-RemoteTestJson $statePath $completeState
    $completed = Invoke-RemotePublication `
        -StatePath $statePath `
        -Assets $assetInventory
    Assert-RemoteTest (
        $completed.resumed_draft -and
        $completed.uploaded_asset_count -eq 0 -and
        $completed.visible_verified
    ) "exact complete draft did not become visible without re-upload"

    foreach ($integrityCase in @(
        "digest_mismatch",
        "missing_digest",
        "size_mismatch"
    )) {
        $remoteAssets = @($assetInventory | ForEach-Object {
            New-RemoteAsset $_
        })
        switch ($integrityCase) {
            "digest_mismatch" {
                $remoteAssets[0].digest = "sha256:$('f' * 64)"
            }
            "missing_digest" {
                $remoteAssets[0].PSObject.Properties.Remove("digest")
            }
            "size_mismatch" {
                $remoteAssets[0].size = [long] $remoteAssets[0].size + 1
            }
        }
        $integrityState = New-RemoteTestState
        $integrityState.releases = @(
            New-RemoteRelease -Assets $remoteAssets
        )
        Write-RemoteTestJson $statePath $integrityState
        Assert-RemoteRejected -Context $integrityCase -Action {
            Invoke-RemotePublication `
                -StatePath $statePath `
                -Assets $assetInventory
        }
    }

    $extraState = New-RemoteTestState
    $extraState.releases = @(
        New-RemoteRelease -Assets (
            $exactRemoteAssets + [pscustomobject]@{
                name = "unexpected.bin"
                size = 1
                digest = "sha256:$('a' * 64)"
                state = "uploaded"
            }
        )
    )
    Write-RemoteTestJson $statePath $extraState
    Assert-RemoteRejected -Context "extra remote asset" -Action {
        Invoke-RemotePublication -StatePath $statePath -Assets $assetInventory
    }

    $duplicateState = New-RemoteTestState
    $duplicateState.releases = @(
        New-RemoteRelease -Assets (
            $exactRemoteAssets + (New-RemoteAsset $assetInventory[0])
        )
    )
    Write-RemoteTestJson $statePath $duplicateState
    Assert-RemoteRejected -Context "duplicate remote asset" -Action {
        Invoke-RemotePublication -StatePath $statePath -Assets $assetInventory
    }

    $duplicateTagState = New-RemoteTestState
    $duplicateTagState.releases = @(
        (New-RemoteRelease -Assets @()),
        ([pscustomobject][ordered]@{
            id = 7002
            tag_name = "v1.2.3"
            draft = $true
            prerelease = $true
            assets = @()
        })
    )
    Write-RemoteTestJson $statePath $duplicateTagState
    Assert-RemoteRejected -Context "duplicate exact release tag" -Action {
        Invoke-RemotePublication -StatePath $statePath -Assets $assetInventory
    }

    $visibleState = New-RemoteTestState
    $visibleState.releases = @(
        New-RemoteRelease -Assets $exactRemoteAssets -Draft $false
    )
    Write-RemoteTestJson $statePath $visibleState
    Assert-RemoteRejected -Context "existing non-draft release" -Action {
        Invoke-RemotePublication -StatePath $statePath -Assets $assetInventory
    }

    foreach ($releaseError in @("auth", "malformed")) {
        $errorState = New-RemoteTestState
        if ($releaseError -ceq "auth") {
            $errorState.fail_release_list = $true
        }
        else {
            $errorState.malformed_release_list = $true
        }
        Write-RemoteTestJson $statePath $errorState
        Assert-RemoteRejected -Context "$releaseError release inventory" -Action {
            Invoke-RemotePublication `
                -StatePath $statePath `
                -Assets $assetInventory
        }
    }

    foreach ($visibilityError in @("auth", "malformed")) {
        $visibilityState = New-RemoteTestState
        $visibilityState.releases = @(
            New-RemoteRelease -Assets $exactRemoteAssets
        )
        if ($visibilityError -ceq "auth") {
            $visibilityState.fail_visibility = $true
        }
        else {
            $visibilityState.malformed_visibility = $true
        }
        Write-RemoteTestJson $statePath $visibilityState
        Assert-RemoteRejected `
            -Context "$visibilityError visibility response" `
            -Action {
                Invoke-RemotePublication `
                    -StatePath $statePath `
                    -Assets $assetInventory
            }
        $afterVisibilityError = Get-Content -LiteralPath $statePath -Raw |
            ConvertFrom-Json -Depth 30
        Assert-RemoteTest (
            $afterVisibilityError.releases[0].draft -eq $true
        ) "visibility error exposed a release in the fake remote"
    }

    function Invoke-TagResolution([object] $State) {
        Write-RemoteTestJson $statePath $State
        $prior = [Environment]::GetEnvironmentVariable(
            "RUSTY_FLEET_FAKE_GH_STATE",
            [EnvironmentVariableTarget]::Process
        )
        try {
            [Environment]::SetEnvironmentVariable(
                "RUSTY_FLEET_FAKE_GH_STATE",
                $statePath,
                [EnvironmentVariableTarget]::Process
            )
            Resolve-RustyFleetRemoteTagCommit `
                -GitHubRepository "MesmerPrism/rusty-fleet" `
                -Tag "v1.2.3" `
                -ExpectedSourceRevision $script:sourceRevision `
                -GhExecutable $script:fakeGhPath
        }
        finally {
            [Environment]::SetEnvironmentVariable(
                "RUSTY_FLEET_FAKE_GH_STATE",
                $prior,
                [EnvironmentVariableTarget]::Process
            )
        }
    }

    $lightweight = New-RemoteTestState
    Assert-RemoteTest (
        (Invoke-TagResolution $lightweight) -ceq $script:sourceRevision
    ) "lightweight remote tag did not resolve exactly"

    $annotatedSha = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    $nestedSha = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    $annotated = New-RemoteTestState
    $annotated.ref_type = "tag"
    $annotated.ref_sha = $annotatedSha
    $annotated.tag_objects = @(
        [ordered]@{
            sha = $annotatedSha
            tag = "v1.2.3"
            target_type = "tag"
            target_sha = $nestedSha
        },
        [ordered]@{
            sha = $nestedSha
            tag = "nested-authority"
            target_type = "commit"
            target_sha = $script:sourceRevision
        }
    )
    Assert-RemoteTest (
        (Invoke-TagResolution $annotated) -ceq $script:sourceRevision
    ) "bounded annotated remote tag chain did not resolve exactly"

    $cycle = New-RemoteTestState
    $cycle.ref_type = "tag"
    $cycle.ref_sha = $annotatedSha
    $cycle.tag_objects = @(
        [ordered]@{
            sha = $annotatedSha
            tag = "v1.2.3"
            target_type = "tag"
            target_sha = $nestedSha
        },
        [ordered]@{
            sha = $nestedSha
            tag = "cycle"
            target_type = "tag"
            target_sha = $annotatedSha
        }
    )
    Assert-RemoteRejected -Context "remote tag cycle" -Action {
        Invoke-TagResolution $cycle
    }

    $deep = New-RemoteTestState
    $deep.ref_type = "tag"
    $deep.ref_sha = "0000000000000000000000000000000000000001"
    $deep.tag_objects = @(
        for ($index = 1; $index -le 9; $index++) {
            [ordered]@{
                sha = $index.ToString("x40")
                tag = if ($index -eq 1) { "v1.2.3" } else { "nested-$index" }
                target_type = if ($index -eq 9) { "commit" } else { "tag" }
                target_sha = if ($index -eq 9) {
                    $script:sourceRevision
                }
                else {
                    ($index + 1).ToString("x40")
                }
            }
        }
    )
    Assert-RemoteRejected -Context "remote tag depth" -Action {
        Invoke-TagResolution $deep
    }

    foreach ($tagError in @("auth", "malformed")) {
        $tagState = New-RemoteTestState
        if ($tagError -ceq "auth") {
            $tagState.fail_tag = $true
        }
        else {
            $tagState.malformed_tag = $true
        }
        Assert-RemoteRejected -Context "$tagError remote tag" -Action {
            Invoke-TagResolution $tagState
        }
    }

    $moved = New-RemoteTestState
    $moved.move_on_tag_check = 3
    Write-RemoteTestJson $statePath $moved
    Assert-RemoteRejected -Context "remote tag move before visibility" -Action {
        Invoke-RemotePublication `
            -StatePath $statePath `
            -Assets $assetInventory
    }
    $movedState = Get-Content -LiteralPath $statePath -Raw |
        ConvertFrom-Json -Depth 30
    Assert-RemoteTest (
        @($movedState.releases).Count -eq 1 -and
        $movedState.releases[0].draft -eq $true -and
        @($movedState.releases[0].assets).Count -eq $assetInventory.Count
    ) "moved remote tag did not leave the verified upload hidden as a draft"

    $movedAfterVisibility = New-RemoteTestState
    $movedAfterVisibility.move_on_tag_check = 4
    Write-RemoteTestJson $statePath $movedAfterVisibility
    Assert-RemoteRejected `
        -Context "remote tag move after visibility" `
        -Action {
            Invoke-RemotePublication `
                -StatePath $statePath `
                -Assets $assetInventory
        }
    $movedAfterVisibilityState = (
        Get-Content -LiteralPath $statePath -Raw |
            ConvertFrom-Json -Depth 30
    )
    Assert-RemoteTest (
        @($movedAfterVisibilityState.releases).Count -eq 1 -and
        $movedAfterVisibilityState.releases[0].draft -eq $false -and
        @($movedAfterVisibilityState.releases[0].assets).Count -eq
            $assetInventory.Count -and
        [int] $movedAfterVisibilityState.tag_check_count -eq 4
    ) "terminal remote tag move was not observed after visibility verification"

    [ordered]@{
        schema = "rusty.fleet.windows_publication_remote_test.v1"
        result = "pass"
        create_verify_visible = $true
        partial_create_resumed = $true
        partial_upload_resumed = $true
        complete_draft_resumed_without_upload = $true
        digest_mismatch_rejected = $true
        missing_digest_rejected = $true
        size_mismatch_rejected = $true
        extra_duplicate_tag_and_visible_release_rejected = $true
        release_auth_and_malformed_json_rejected = $true
        visibility_auth_and_malformed_json_rejected = $true
        lightweight_and_annotated_tags_verified = $true
        pre_and_post_visibility_tag_moves_rejected = $true
        tag_cycle_depth_auth_and_malformed_rejected = $true
    } | ConvertTo-Json -Depth 5
}
finally {
    [Environment]::SetEnvironmentVariable(
        "RUSTY_FLEET_FAKE_GH_STATE",
        $priorFakeState,
        [EnvironmentVariableTarget]::Process
    )
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}
