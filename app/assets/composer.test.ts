// @vitest-environment happy-dom
import { expect, test, vi } from "vitest";
import { initComposer } from "./composer";

test("the send shortcut submits Quick task", () => {
    const form = document.createElement("form");
    form.innerHTML = `
        <textarea id="composer-message"></textarea>
        <button type="submit" name="mode" value="quick">Send</button>
        <button type="submit" name="mode" value="configured">Send with workflow</button>
    `;
    const textarea = form.querySelector("textarea")!;
    const quick = form.querySelector<HTMLButtonElement>('[value="quick"]')!;
    const requestSubmit = vi.fn();
    form.requestSubmit = requestSubmit;

    initComposer(form, { signal: new AbortController().signal });
    textarea.dispatchEvent(
        new KeyboardEvent("keydown", {
            key: "Enter",
            ctrlKey: true,
            bubbles: true,
            cancelable: true,
        }),
    );

    expect(requestSubmit).toHaveBeenCalledOnce();
    expect(requestSubmit).toHaveBeenCalledWith(quick);
});
