# Qobuz Downloader

I download purchased albums and tracks through browser authorisation. A
streaming subscription is not required. The downloader has its own account.

I fetch or enter application credentials in Account settings, open browser
sign-in and check the result. After selecting an export directory, I browse
purchases, inspect an album, choose an entitled format and download an album or
one track. Progress and cancellation stay in the download view.

I stream into a temporary `.part` file, check its audio container, then rename
it after completion. I request `folder_write` for the selected directory; a
path in an RPC request does not grant filesystem access. Library scanning is
left to the player.

I keep credentials and the session in an atomic `account.json` in the private
data directory, without a keyring or application-level encryption. The account
password stays in the browser. I never log credentials or signed download URLs.

## Runtime directories

The worker waits for `initialize` before opening account state. The host must
provide absolute `data_dir` and `cache_dir` paths. Downloads use the optional
`export_dir` exactly as supplied, including a descriptor path under Flatpak.
The download view reports an unavailable destination until a folder is selected.
Directory changes require a worker restart. Each platform archive supports both
native and Flatpak hosts with this directory contract.

## Build and checks

I build with `tools/build-native`. On Linux, I set `PLUGIN_BUILD_LOCK` to the
workstation's shared Rust build lock. The build uses `Cargo.lock` offline,
remaps build paths and writes the native package under `dist/`.

I run workspace tests under the same build lock and check the packaged worker
with `python3 tools/smoke /path/to/plugin`. Account-free checks use a temporary
profile; actual purchases require an authorised account and a selected folder.

## macOS

MacOS is provided but not tested, I don't have a macbook

## Licence

I distribute the plugin under GPL-3.0-or-later. I retain the upstream MIT notices
for the API, data types and media helpers. See `COPYING`, `NOTICE` and `crates/`.
