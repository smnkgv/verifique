# verifique

-   Never name a private client, platform or project anywhere in this repository — code, docs,
    commits, release notes or issues — because it is public. Examples use made-up apps.
-   Keep verifique universal: it knows nothing about a project's language, framework or tools. What
    one project needs goes in that project's own commands in `verifique.yaml`, never in verifique.
-   Write OS-specific code only in the platform layer; the rest of the code calls it without knowing
    the OS, because macOS, Linux and Windows are equals. Docs, install paths and releases treat the
    three the same.

## Releasing

Release only when the user says so. Then:

1. Bump `version` in the root `Cargo.toml`, commit "Release x.y.z", push `main`.
2. Run the `release` workflow: it builds every platform into a draft release.
3. Add the notes and publish it.
4. Update `Formula/verifique.rb` and `Casks/verifique-panel.rb` in `smnkgv/homebrew-verifique`, and
   `bucket/verifique.json` in `smnkgv/scoop-verifique`, from the release's `SHA256SUMS`.
5. Run the `publish` workflow in `smnkgv/verifique-packages` for the apt, dnf and pacman
   repositories.
