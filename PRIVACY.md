# Privacy Policy

**Capcove** values your privacy. The application does not collect, store on a central server, or share any personal user data with the developer or any third party we control.

## 1. Data Collection and Storage

Capcove is a desktop client. Recordings, settings, tags, and upload history are stored exclusively on your own device — in your chosen recordings folder and in `%APPDATA%\dev.xacnio.capcove\`. Neither the developer nor any server we operate can access this data, because no such server exists; Capcove has no backend.

## 2. Third-Party Services

The application can communicate directly with the following external services, only for features you enable:

- **Google Drive:** If you connect a Google account, Capcove requests OAuth access to Google Drive using only the `drive.file` scope, which limits access strictly to files and folders created by the app itself. Capcove cannot read or access any other content in your Drive. The connection uses OAuth 2.0 (PKCE) directly between your device and Google — no Capcove server is involved. Access tokens are stored only in your OS credential store (Windows Credential Manager, macOS Keychain, or Linux Secret Service), never transmitted elsewhere. By default, Capcove uses its own OAuth client; you can supply your own Google Cloud OAuth client instead in Settings → Advanced.
- **YouTube:** If you connect a YouTube channel (which can use the same or a different Google account as Drive), Capcove requests the `youtube.upload` and `youtube` OAuth scopes so it can create private/unlisted/public live broadcasts for the "stream instead of recording locally" feature, and upload exported clips from the built-in editor. As with Drive, these scopes are technically broad enough to manage your whole channel, but Capcove's own code only creates or ends broadcasts and uploads the videos you explicitly start or export. Tokens are stored the same way as Drive tokens — in your OS credential store, in a separate slot — and never transmitted elsewhere.
- **Discord:** Capcove's Game Detection feature periodically fetches a public game-detection catalog (executable names, display names, and icon URLs) from Discord's API, cached on disk and refreshed at most once a week. This is an anonymous, unauthenticated request: no account connection, and no information about which games you actually play is sent to Discord.
- **GitHub:** Capcove checks `github.com` for release information to power the in-app update checker and updater. This is an anonymous, unauthenticated request — no personal data is sent.
- **Google Translate:** If you open the in-app Terms of Service or Privacy Policy viewer and the app isn't in English, you can request a machine-translated view. This sends the document text (not personal data) to Google's public translation endpoint and is only triggered by your explicit click — it never happens automatically. The translation is for convenience only; the English text remains authoritative.

Capcove is not affiliated with or endorsed by Google, YouTube, Discord, or GitHub. Use of these services is also subject to their own privacy policies and terms.

## 3. Automatic Updates

The application periodically checks GitHub for new releases (if enabled in Settings). This request contains no personal data and is used solely to determine whether a newer version is available for download.

## 4. Data Protection

All sensitive data handled by Capcove is protected as follows:

- **OAuth tokens:** Google OAuth access and refresh tokens are stored exclusively in your operating system's secure credential store (Windows Credential Manager, macOS Keychain, or Linux Secret Service). They are never written to plain text files, logs, or transmitted to any server other than Google's own OAuth endpoints.
- **Network connections:** All connections to Google APIs use HTTPS/TLS. No Capcove-operated server sits between your device and these services.
- **No developer access:** Because Capcove has no backend server, the developer has no technical ability to access, intercept, or read any of your data.

## 5. Google User Data Retention and Deletion

Capcove stores the following Google-related data locally on your device:

- **OAuth tokens** (access token and refresh token) in your OS credential store.
- **Uploaded file records** (`uploaded.json`, `pending_uploads.json`) — a map of local file names to their Google Drive file IDs and the pending upload queue, used to avoid re-uploading the same file.
- **Metadata and icon cache** — synced copies of recording metadata and app icons stored in the Capcove subfolder on your Drive.

**Retention:** This data is kept for as long as your Google account remains connected in Capcove.

**Deletion:** You can remove all locally stored Google user data at any time by disconnecting your Google account in Settings → Google Drive → Disconnect. This action deletes the OAuth tokens from your OS credential store and clears the local upload records. Files already uploaded to your Google Drive are not deleted by this action — you can remove them directly from Google Drive. Uninstalling the application also removes all locally stored app data including tokens and records. You can also revoke Capcove's access to your Google account at any time from [Google's app permissions page](https://myaccount.google.com/permissions).

## 6. Analytics and Telemetry

Capcove does not include any analytics, telemetry, crash reporting, or tracking code. The developer has no visibility into how you use the app, what you capture, or which accounts you connect.

## 7. Contact

If you have any questions about this policy or the app's privacy practices, please open an [Issue](https://github.com/xacnio/capcove/issues) on the GitHub repository.

---
*Last updated: July 13, 2026*
