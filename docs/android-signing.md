# Android signing key

The dev APKs (`dev.horizon.android.dev`) are only updatable in place when every
build carries **the same certificate and a higher versionCode**. Android refuses
an update whose certificate differs, so a key that changes per build forces a
reinstall every time. This page sets up the one key the workflow reuses.

## 1. Generate the key (once)

Actions → **generate-signing-key** → *Run workflow*. It creates one keystore with
`keytool`: RSA 4096, valid 100 years, alias `horizon`, a random strong password.
It never prints the keystore, the passwords or the base64 — they only exist in the
artifact.

When the run finishes, open it and download the **horizon-signing-key** artifact
(expires after 1 day). It holds three files:

- `keystore.jks` — the keystore itself (PKCS12)
- `keystore.jks.base64` — the same file, base64 encoded
- `credentials.txt` — the alias and the password

## 2. Store the values as repository secrets

Settings → Secrets and variables → Actions → *New repository secret*, and add:

| secret | value |
|---|---|
| `ANDROID_KEYSTORE_BASE64` | the whole contents of `keystore.jks.base64` |
| `ANDROID_KEYSTORE_PASSWORD` | the password from `credentials.txt` |
| `ANDROID_KEY_ALIAS` | `horizon` |

`ANDROID_DEV_KEYSTORE_BASE64` is optional: when set (a dev key, password
`android`, alias `androiddebugkey`) it takes precedence over the release key.

## 3. Delete the artifact and keep a private backup

**Delete the artifact** once the secrets are set. Keep `keystore.jks` and
`credentials.txt` in a private backup outside the repository (a password manager
or an offline drive). **If the keystore is lost you cannot update the installed
dev app again** — you would generate a new key and reinstall. Never commit a
keystore, and never attach it to an issue, release or message.

## 4. First run after the key changes

The already installed dev build is signed with a different key, so **the first
install with the new key needs a one-time uninstall** of the old `dev.horizon.android.dev`
app (its worlds and accounts belong to that app and are lost with it). From then
on, every dev APK signed with this key updates in place.

## When no secret is set

The dev-APK job still builds, but signed with a **throwaway** debug key:

- the log gets a `::warning::` and the run summary a **NO-STABLE-KEY** banner,
- the APK is named `Horizon-Android-dev.<run>.<sha>-<abi>-NO-STABLE-KEY.apk`,
- such an APK **cannot update an installed build**.

Once `ANDROID_KEYSTORE_BASE64` (or `ANDROID_DEV_KEYSTORE_BASE64`) is set, the
build uses it and the `-NO-STABLE-KEY` suffix disappears.

## versionCode

The dev build's `versionCode` is `1000 + <Actions run number>` (see
`.github/workflows/android-dev.yml`), so each build is newer than the last.
Do not reset the workflow's run counter.
