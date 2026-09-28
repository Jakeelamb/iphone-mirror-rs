# Prepare an iPhone

This is separate from building the viewer. The known working combination is an
iPhone 15 on iOS 27 with developer image build `27A5228h`. That is a test result,
not a promise of compatibility with every phone or a proven minimum iOS version.

Use one unlocked phone connected directly by a USB data cable during setup.
Close any running mirror before pairing or changing developer images.

## Install the setup tool

The viewer itself is native Rust. These preparation commands use
[pymobiledevice3 11.13.1](https://github.com/doronz88/pymobiledevice3/tree/v11.13.1),
the version used during this project's setup. With [uv](https://docs.astral.sh/uv/):

```sh
uv tool install 'pymobiledevice3==11.13.1'
pymobiledevice3 --help
```

If the command is not found, add uv's tool directory to PATH using
`uv tool update-shell`, then open a new terminal. Linux USB access also needs
`usbmuxd` installed and available. The tool downloads Python dependencies; it is
not included in this repository.

## 1. Trust the computer

```sh
pymobiledevice3 usbmux list --usb
pymobiledevice3 lockdown pair
```

Accept Trust and enter the passcode **on the phone** if prompted. Pairing saves
credentials on this computer. Discovery output can contain device identifiers;
redact them before sharing diagnostics.

## 2. Enable Developer Mode

If the setting is hidden, reveal it:

```sh
pymobiledevice3 amfi reveal-developer-mode
```

On the phone open **Settings → Privacy & Security → Developer Mode**, enable it,
and follow the restart and confirmation prompts. The command only reveals the
setting; it does not enable it. Reconnect and unlock the phone afterward.

## 3. Prepare developer services

Inspect the current image first:

```sh
pymobiledevice3 mounter list
pymobiledevice3 mounter auto-mount --help
```

When an image is needed, explicitly download/mount it with:

```sh
pymobiledevice3 mounter auto-mount
```

This can download an Apple developer image and request a personalization ticket.
Image compatibility matters: an image being mounted does not prove the display
and input services are usable. Check the reported iOS/image versions before
replacing an existing image. After a phone restart, check image availability
again. The Rust viewer never mounts or replaces images itself.

## 4. Pair for Wi-Fi

While USB is still connected and trusted:

```sh
pymobiledevice3 lockdown remotepairing --pair
```

This saves a separate CoreDevice credential. Put the phone and computer on the
same local network, unplug USB to verify actual Wi-Fi, then run the Rust viewer:

```sh
iphone-mirror-rs --connection wifi
```

For USB, use `--connection usb` after the trust/developer preparation above;
separate Wi-Fi pairing is not needed. USB support in this Rust viewer remains
unverified on a live phone.

## Saved records

pymobiledevice3 uses `~/.pymobiledevice3/` when that legacy directory exists;
otherwise it uses `${XDG_DATA_HOME:-$HOME/.local/share}/pymobiledevice3/`.
The Rust viewer follows that lookup and reads `remote_*.plist` records without
changing them. Use `--pairing-file /path/to/remote_DEVICE.plist` for another
location or `--serial ID` when multiple phones are paired.

USB trust records are managed separately by usbmuxd, typically under
`/var/lib/lockdown/`. Both kinds contain credentials: do not commit, upload, or
attach them to an issue. Uninstalling the viewer does not revoke trust or delete
these records. Keep the setup tool and pairing directory on the same host;
some records derive their host identifier from its hostname.

For upstream setup details, see the
[reference project's phone guide](https://github.com/daniellemky/omarchy-iphone-mirror/blob/main/docs/phone-setup.md).
Its installer and launcher commands belong to that project, not this viewer.
