# cosmic-applet-plex

A native COSMIC panel applet for Plex Media Server with server status, library scanning, and Now Playing.
- Checks server status every 10 seconds and when opening the popup.
- Shows a connection-status icon at the far right of the **Plex** header,
  with the status description available on hover.
- Provides three centered icon buttons with hover tooltips, from left to right:
  **Rescan folders**, **Open Plex**, and **Plex token**.
- Opens the local Plex web interface in your browser.
- Requests a scan of all Plex libraries.
- Shows active playback below the action buttons: poster, title, device,
  playback state, position, and duration. Multiple sessions are scrollable.
- Refreshes playback every five seconds while the popup is open. When Plex
  returns no active sessions, the playback section and its separator are hidden.
- Follows the COSMIC **Applets** frosted-glass setting for the popup background.
- Does not change the Plex service, startup behavior, or server settings.

## Build and Install

Requires Rust 1.93 or newer, `pkg-config`, a C compiler, and development
libraries for Wayland and xkbcommon. On Pop!\_OS/Ubuntu:

```sh
sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev
make test
make install
```

`make install` installs into `~/.local` without administrator privileges.
Add **Plex** through the COSMIC panel settings.
After updating, remove and re-add the applet to load the new binary.
`make uninstall` removes the applet but keeps the saved token.

Git dependencies are pinned in Cargo.lock; builds use `--locked`.

## Plex Token

Checking server status does not require a token. Scanning and viewing active
playback require a token with the corresponding permissions on this server.

Click the settings icon (**Plex token**), enter the token in the masked
field, and click **Save token**. Never share the token in chat or commit it
to git.

The token is stored at `$XDG_CONFIG_HOME/cosmic-applet-plex/token`, or
`~/.config/cosmic-applet-plex/token` when XDG_CONFIG_HOME is not set.
Directory permissions are `0700` and file permissions are `0600`.
This is a local plaintext file, not an encrypted keyring. Requests send the
token in the `X-Plex-Token` header, not in the URL.

To find your token:

1. Open a movie or episode in Plex Web.
2. Open its three-dot menu, then choose **Get Info > View XML**.
3. Find `X-Plex-Token=` in the XML page's address.
4. Copy only the value after `=`, up to the next `&` or the end of the URL.
5. Paste it into the applet's token field and save it.

Do not share the XML URL: it contains your token. Delete the local token
file to remove the applet's stored access. This does not revoke the token
in Plex itself.

## Rescan Folders

The applet requests `GET /library/sections`, then sends
`GET /library/sections/{key}/refresh` for each library. This is a regular scan
for new or changed content, not a forced metadata refresh, trash cleanup,
or server restart.

When all scan requests are accepted, the scan icon turns green for five
seconds instead of displaying a success message. This confirms acceptance,
not that Plex has finished scanning. Partial failures show the number of
accepted and failed requests. The scan button is disabled while the server
is offline or scan requests are being sent.

## Now Playing

The applet reads `GET /status/sessions` using the saved token. Movies,
episodes, and music show their title, poster, playback device, state
(such as **Playing** or **Paused**), and position / duration.
Episode titles include the series, season, and episode. Music includes
the artist when available.

Posters use Plex's thumbnail endpoint with the token in an HTTP header.
Loaded posters are reused between refreshes; unavailable artwork uses a
placeholder. Empty playback has no section or separator; authorization and
connection errors remain visible. Playback information is read-only: the applet
does not pause, seek, or otherwise control any player.
