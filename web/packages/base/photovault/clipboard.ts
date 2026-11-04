/**
 * PHOTOVAULT: clipboard - copy text on plain http:// too.
 *
 * Self-hosted servers are often opened as http://<lan-ip>:<port>, which is
 * not a secure context, so `navigator.clipboard` is missing there and Ente's
 * copy buttons silently did nothing. Try the async Clipboard API, then the
 * legacy `execCommand("copy")`, and as a last resort show the text in a
 * prompt so the user can copy it by hand.
 *
 * @returns `true` when the text was put on the clipboard.
 */
export const copyTextToClipboard = async (text: string): Promise<boolean> => {
    if (window.isSecureContext && typeof navigator.clipboard !== "undefined") {
        try {
            await navigator.clipboard.writeText(text);
            return true;
        } catch {
            // Fall through to the legacy path (e.g. permission denied).
        }
    }
    if (copyWithExecCommand(text)) return true;
    window.prompt("Copy this text (Ctrl+C / Cmd+C), then press OK:", text);
    return false;
};

const copyWithExecCommand = (text: string) => {
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.setAttribute("readonly", "");
    textarea.style.position = "fixed";
    textarea.style.top = "0";
    textarea.style.left = "0";
    textarea.style.opacity = "0";
    document.body.appendChild(textarea);
    const selection = document.getSelection();
    const previous = selection?.rangeCount ? selection.getRangeAt(0) : null;
    textarea.select();
    textarea.setSelectionRange(0, text.length);
    let ok: boolean;
    try {
        // eslint-disable-next-line @typescript-eslint/no-deprecated
        ok = document.execCommand("copy");
    } catch {
        ok = false;
    }
    document.body.removeChild(textarea);
    if (previous && selection) {
        selection.removeAllRanges();
        selection.addRange(previous);
    }
    return ok;
};
