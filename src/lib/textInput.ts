// macOS text "fixing" (capital first letter, autocorrect, smart quotes,
// spelling) breaks what this app's fields hold: user names, hosts, paths,
// commands, keys. WKWebView applies it to every text field unless the field
// opts out, so every field opts out: when it gets focus, before anything is
// typed. One listener covers fields in dialogs, menus and future views too.

const OFF: [string, string][] = [
  ["autocapitalize", "off"],
  ["autocorrect", "off"],
  ["autocomplete", "off"],
  ["spellcheck", "false"],
];

export function plain(el: Element) {
  for (const [name, value] of OFF) {
    // A field that asks for something else on purpose keeps it.
    if (!el.hasAttribute(name)) el.setAttribute(name, value);
  }
}

function isTextField(el: EventTarget | null): el is HTMLInputElement | HTMLTextAreaElement {
  return el instanceof HTMLTextAreaElement || (el instanceof HTMLInputElement && !["checkbox", "radio", "range", "button", "submit", "file", "color"].includes(el.type));
}

/** Turn macOS text fixing off for every text field in the app. Call once at startup. */
export function disableTextFixing() {
  document.documentElement.setAttribute("spellcheck", "false");
  // As fields appear, and again on focus (in case a view set its own attributes later).
  const scan = (root: ParentNode) => root.querySelectorAll("input, textarea").forEach((el) => isTextField(el) && plain(el));
  scan(document);
  new MutationObserver((changes) => {
    for (const c of changes) c.addedNodes.forEach((n) => n instanceof Element && (isTextField(n) ? plain(n) : scan(n)));
  }).observe(document.documentElement, { childList: true, subtree: true });
  document.addEventListener(
    "focusin",
    (e) => {
      if (isTextField(e.target)) plain(e.target);
    },
    true,
  );
}
