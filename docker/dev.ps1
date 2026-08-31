<#
.SYNOPSIS
  The F development environment, from Windows.

.DESCRIPTION
  Wraps docker compose so that the everyday commands are one word. Docker is
  the only prerequisite: no Rust, no QEMU, nothing on PATH.

    .\docker\dev.ps1 doctor        # is this machine ready, and what will be slow
    .\docker\dev.ps1 build         # build the image (once, and after a pin bump)
    .\docker\dev.ps1 shell         # an interactive shell in /work

    .\docker\dev.ps1 verify        # cargo xtask verify — the whole local loop
    .\docker\dev.ps1 lint          # cargo xtask lint
    .\docker\dev.ps1 test          # cargo xtask test
    .\docker\dev.ps1 run           # cargo xtask run   (kernel in QEMU)
    .\docker\dev.ps1 claims        # cargo xtask claims
    .\docker\dev.ps1 coverage      # cargo xtask coverage

    .\docker\dev.ps1 x <args...>   # any other xtask verb
    .\docker\dev.ps1 cargo <args>  # cargo, directly
    .\docker\dev.ps1 full <args>   # the same, in the full image (cargo-deny etc.)

    .\docker\dev.ps1 export        # copy target/ out of its volume, to .\target-export
    .\docker\dev.ps1 clean         # drop the build and cache volumes

  Written for Windows PowerShell 5.1 as well as PowerShell 7, because 5.1 is
  what Windows 10 ships and an environment script that needs its own
  environment installed first is not much of an environment script.
#>

[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [string]$Command = "help",

    [Parameter(Position = 1, ValueFromRemainingArguments = $true)]
    [string[]]$Rest
)

$ErrorActionPreference = "Stop"

$RepoRoot   = Split-Path -Parent $PSScriptRoot
$ComposeFile = Join-Path $PSScriptRoot "compose.yaml"

function Fail([string]$Message) {
    Write-Host "dev: $Message" -ForegroundColor Red
    exit 1
}

function Require-Docker {
    if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
        Fail "docker was not found on PATH. Install Docker Desktop and reopen this shell."
    }
    docker info 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Fail "the Docker daemon is not responding. Is Docker Desktop running?"
    }
}

# The parameter is `$ComposeArgs` and not the obvious `$Args` because `$Args`
# is an automatic variable: a parameter of that name binds nothing, silently,
# and every command below degenerates into a bare `docker compose -f <file>`
# that prints the usage screen and exits 0. Do not rename it back.
function Compose {
    param([string[]]$ComposeArgs)
    & docker compose -f $ComposeFile @ComposeArgs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

# One-shot command in a throwaway container. `--rm` because a development
# container that accumulates state is a development machine again.
function Exec {
    param([string]$Service, [string[]]$ComposeArgs)
    Compose -ComposeArgs (@("run", "--rm", $Service) + $ComposeArgs)
}

function Xtask {
    param([string]$Service, [string[]]$ComposeArgs)
    Exec -Service $Service -ComposeArgs (@("cargo", "xtask") + $ComposeArgs)
}

switch ($Command.ToLower()) {

    "doctor" {
        Write-Host "F development environment — readiness" -ForegroundColor Cyan
        Write-Host ""

        $dockerOk = $null -ne (Get-Command docker -ErrorAction SilentlyContinue)
        Write-Host ("  docker on PATH         : " + $(if ($dockerOk) { "yes" } else { "NO — install Docker Desktop" }))
        if ($dockerOk) {
            Write-Host ("  docker version         : " + (docker version --format '{{.Server.Version}}' 2>$null))
        }

        $wsl = $null -ne (Get-Command wsl -ErrorAction SilentlyContinue)
        Write-Host ("  wsl present            : " + $(if ($wsl) { "yes" } else { "no" }))

        # Hardware acceleration for QEMU needs /dev/kvm inside WSL2, which needs
        # nested virtualisation. Its absence is not a problem — it is a speed
        # limit, and a reason not to collect timings here.
        $kvm = "no"
        if ($wsl) {
            wsl -e test -e /dev/kvm 2>$null
            if ($LASTEXITCODE -eq 0) { $kvm = "yes" }
        }
        Write-Host ("  /dev/kvm in WSL2       : " + $kvm)
        if ($kvm -ne "yes") {
            Write-Host "      QEMU will run in software emulation (TCG). Correctness is unaffected;" -ForegroundColor DarkYellow
            Write-Host "      speed is roughly an order of magnitude down, and timing numbers" -ForegroundColor DarkYellow
            Write-Host "      collected here are not claims. See docker/README.md." -ForegroundColor DarkYellow
        }

        $image = docker images -q f-dev:latest 2>$null
        Write-Host ("  image f-dev:latest     : " + $(if ($image) { "built" } else { "not built — run: .\docker\dev.ps1 build" }))

        Write-Host ("  repository path        : " + $RepoRoot)
        if ($RepoRoot -match '[^\x00-\x7F]') {
            Write-Host "      This path contains non-ASCII characters. Docker Desktop handles that," -ForegroundColor DarkYellow
            Write-Host "      but if a bind mount ever fails mysteriously, this is the first thing" -ForegroundColor DarkYellow
            Write-Host "      to rule out — clone into a WSL2 path instead. See docker/README.md." -ForegroundColor DarkYellow
        }
        Write-Host ""
        Write-Host "  Next: .\docker\dev.ps1 build   then   .\docker\dev.ps1 lint" -ForegroundColor Cyan
    }

    "build" {
        Require-Docker
        $service = "dev"
        if ($Rest -and $Rest[0] -eq "full") { $service = "full" }
        Compose -ComposeArgs @("build", $service)
        Write-Host ""
        Write-Host "built. Try: .\docker\dev.ps1 lint" -ForegroundColor Green
    }

    "shell"    { Require-Docker; Exec  -Service "dev"  -ComposeArgs @("/bin/bash") }

    # `verify` is the command CLAUDE.md tells a session to run before asking for
    # review, and it was the one verb this wrapper did not have — so the
    # supported environment could not run the supported check without falling
    # through to `x`. Added when the README started telling a newcomer to use it.
    "verify"   { Require-Docker; Xtask -Service "dev"  -ComposeArgs @("verify") }
    "lint"     { Require-Docker; Xtask -Service "dev"  -ComposeArgs @("lint") }
    "test"     { Require-Docker; Xtask -Service "dev"  -ComposeArgs @("test") }
    "run"      { Require-Docker; Xtask -Service "dev"  -ComposeArgs @("run") }
    "claims"   { Require-Docker; Xtask -Service "dev"  -ComposeArgs @("claims") }
    "coverage" { Require-Docker; Xtask -Service "dev"  -ComposeArgs @("coverage") }

    "x"        { Require-Docker; Xtask -Service "dev"  -ComposeArgs $Rest }
    "cargo"    { Require-Docker; Exec  -Service "dev"  -ComposeArgs (@("cargo") + $Rest) }
    "full"     { Require-Docker; Exec  -Service "full" -ComposeArgs $Rest }

    "export" {
        Require-Docker
        $dest = Join-Path $RepoRoot "target-export"
        New-Item -ItemType Directory -Force -Path $dest | Out-Null
        Write-Host "copying /work/target out of its volume into $dest ..."
        Compose -ComposeArgs @("run", "--rm", "-v", "${dest}:/export", "dev",
                        "bash", "-lc", "cp -a /work/target/. /export/ && ls -la /export | head")
        Write-Host "done." -ForegroundColor Green
    }

    "clean" {
        Require-Docker
        Write-Host "removing the build and cache volumes (the working tree is untouched)..."
        Compose -ComposeArgs @("down", "-v")
        Write-Host "done. The next build re-downloads the registry index." -ForegroundColor Green
    }

    default {
        Get-Help $PSCommandPath -Detailed
    }
}
