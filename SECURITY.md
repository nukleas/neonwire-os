# Security

## This is a lab device

NEONWIRE runs as root on a tablet that was never designed to be on a network.
Dropbear is pubkey-only, but the rest of the userspace is a busybox initramfs
with no MAC, no updates channel, and no pretence of being a hardened OS. Treat
a unit running this as a toy on a trusted LAN (or a Tailscale tailnet), not as
something you expose to the internet.

Flashing via the unprotected Preloader can brick the device. Dump and hash
before you write. There is no warranty.

## Secrets stay on the tablet

Device-local credentials live on the SD card at `/mnt/sd/linux-lab/` and are
gitignored. Never commit them. A non-exhaustive list:

| File | What |
|------|------|
| `hass.token` / `hass.url` | Home Assistant long-lived access token + base URL |
| `xai.key` | xAI API key (voice STT) |
| `ocint.key` | optional OCINT bearer |
| `zeroclaw/token` | on-device agent pairing token |
| `.gh_token` | GitHub PAT, only if pulling a private release |
| `authorized_keys` | dropbear login keys |
| `dropbear/` | SSH host keys |
| `ts-state/` | Tailscale node state |

The HOUSE app will not talk to Home Assistant until `hass.url` and `hass.token`
are both present. There is no compiled-in fallback URL.

## Vendor material

`reference/` is excluded from this repository on purpose. Firmware blobs,
extracted HALs, full eMMC dumps, and cloned vendor kernel trees are not ours to
publish. The writeups describe how they were used; they do not ship them.

## Reporting

If you find a live credential, a private host, or a vendor blob that slipped
into git, open a **private** GitHub security advisory on this repo (or email
the author) rather than a public issue. Hardware findings and RE notes can be
ordinary issues.
