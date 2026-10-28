# BlazeBypass

A small Windows app for bypassing ad links in bulk with the [bypass.vip](https://bypass.vip) premium API. Paste links one per line, bypass them, and copy the results straight into JDownloader 2.

Built in Rust with [egui](https://github.com/emilk/egui). It's a single portable exe with no installer.

## Features

- Bulk input with one link per line. Duplicates are removed, and `https://` is added when it's missing.
- Bypasses three links at a time and stays under the API's limit of 10 requests per 10 seconds.
- Shows the status of each link. Click a result to copy it.
- **Copy all** puts every bypassed URL on the clipboard, one per line.
- Each run is saved to `results\bypass_<timestamp>.txt` next to the exe.
- **Refresh mode** uses the `/premium/refresh` endpoint to skip cached results. Use it only for links that change often.

## API key

The key is not included in this repo. The app looks for it in this order:

1. The `BYPASS_VIP_API_KEY` environment variable
2. `api_key.txt` next to the exe

If neither is found, the app asks for the key and saves it to `api_key.txt`.

## Build

```bash
cargo build --release
```

The exe is written to `target/release/blaze-bypass.exe`.
