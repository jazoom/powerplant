// @vitest-environment happy-dom
import { expect, test } from "vitest";
import { initComposer } from "./composer";

function composerForm(): HTMLFormElement {
    const form = document.createElement("form");
    form.innerHTML = `<textarea></textarea><button type="submit">Send</button>`;
    return form;
}

function mountComposer(form: HTMLFormElement): () => void {
    const controller = new AbortController();
    initComposer(form, { signal: controller.signal });
    return () => controller.abort();
}

function trackSubmit(form: HTMLFormElement): () => boolean {
    let submitted = false;
    form.requestSubmit = () => {
        submitted = true;
    };
    return () => submitted;
}

function enter(
    target: EventTarget,
    init: Omit<KeyboardEventInit, "key" | "bubbles"> = {},
): void {
    target.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", bubbles: true, ...init }),
    );
}

test("control-enter submits the composer form", () => {
    const form = composerForm();
    const submitted = trackSubmit(form);
    const destroy = mountComposer(form);
    enter(form.querySelector("textarea")!, { ctrlKey: true });
    expect(submitted()).toBe(true);
    destroy();
});

test("command-enter submits the composer form", () => {
    const form = composerForm();
    const submitted = trackSubmit(form);
    mountComposer(form);
    enter(form.querySelector("textarea")!, { metaKey: true });
    expect(submitted()).toBe(true);
});

test("plain enter does not submit", () => {
    const form = composerForm();
    const submitted = trackSubmit(form);
    mountComposer(form);
    enter(form.querySelector("textarea")!);
    expect(submitted()).toBe(false);
});

test("composition enter does not submit", () => {
    const form = composerForm();
    const submitted = trackSubmit(form);
    mountComposer(form);
    enter(form.querySelector("textarea")!, {
        ctrlKey: true,
        isComposing: true,
    });
    expect(submitted()).toBe(false);
});

test("a replaced textarea keeps the shortcut", () => {
    const form = composerForm();
    const submitted = trackSubmit(form);
    mountComposer(form);
    const next = document.createElement("textarea");
    form.querySelector("textarea")!.replaceWith(next);
    enter(next, { ctrlKey: true });
    expect(submitted()).toBe(true);
});
