// Links (editor.md#links): the schemes a link may open with outside the app,
// and whether a URL is safe to keep.

// The schemes a link may open with outside the app: web, email and phone
// (editor.md#links); the host's `open_url` takes the same.
const LINK_SCHEMES = ['http', 'https', 'mailto', 'tel'];

// Whether `url` names one of `LINK_SCHEMES`.
export function isLink(url) {
  const m = /^([a-zA-Z][a-zA-Z0-9+.-]*):/.exec(url.trim());
  return !!m && LINK_SCHEMES.includes(m[1].toLowerCase());
}

// Relative URLs, the usual web schemes and links into the vault (`id:`,
// vault.md#links-in-content) only -- never `javascript:`.
export function isSafeUrl(url) {
  const m = /^([a-zA-Z][a-zA-Z0-9+.-]*):/.exec(url.trim());
  return !m || m[1].toLowerCase() === 'id' || isLink(url);
}
