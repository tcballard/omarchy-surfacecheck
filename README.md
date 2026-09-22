<h1 align="center">Omarchy SurfaceCheck</h1>

<p align="center">
  <a href="https://github.com/tcballard/omarchy-badges"><img src="https://raw.githubusercontent.com/tcballard/omarchy-badges/75975e5b5bf75e7ede3764bcd2950046f7abfe2c/badges/v1/omarchy-plugin.svg" alt="Built for Omarchy: Plugin" height="24"></a>
</p>

**Capture the problem. Keep the evidence.**

SurfaceCheck helps you inspect an Omarchy window or region, compare before and after, and keep the captures and findings together locally. Optional agent review starts only when you request it.

## Everyday use

Choose **Check this surface**, capture the window or region and add a note. Review the measured findings, compare a later capture, or export the local evidence bundle. [Workflow →](GUIDE.md#workflow)

## Install

Omarchy Quattro with Quickshell, Hyprland, grim, slurp and hyprctl; Rust/Cargo for the source build; and a systemd user session.

Install the plugin and its native runtime, then enable it:

```bash
omarchy plugin add https://github.com/tcballard/omarchy-surfacecheck.git --yes
~/.config/omarchy/plugins/tcballard.surfacecheck/scripts/install-user.sh
omarchy plugin enable tcballard.surfacecheck
```

The installer builds the locked Rust workspace and starts the user service. [Update and removal →](GUIDE.md#install-on-omarchy-quattro)

## Update and remove

This plugin also has a native runtime. Follow the [runtime and plugin instructions](GUIDE.md#install-on-omarchy-quattro) when updating or removing it. The shell plugin can be removed with `omarchy plugin remove tcballard.surfacecheck`.

## A few useful details

**v0.1 implementation.** Machine-independent verification is documented; live desktop acceptance remains outstanding. [Acceptance](docs/live-acceptance.md) · [Verification](GUIDE.md#verification)

Captures and notes stay local by default and survive removal. Agent review and external handoff require explicit consent. [Privacy](docs/privacy.md) · [Removal](GUIDE.md#install-on-omarchy-quattro)

[Usage and development guide](GUIDE.md) · [Report a bug](https://github.com/tcballard/omarchy-surfacecheck/issues)

[Licence licensed](LICENSE).

<!-- Preserve links to sections now in the guide. -->
<a id="architecture"></a>
<a id="install-on-omarchy-quattro"></a>
<a id="licence"></a>
<a id="privacy-and-evidence"></a>
<a id="quick-start"></a>
<a id="verification"></a>
<a id="workflow"></a>

[Looking for the previous detailed sections? Open the full guide →](GUIDE.md)
