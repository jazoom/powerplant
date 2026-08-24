// @vitest-environment happy-dom
import { expect, test } from "vitest";
import { initComposer } from "./composer";

test("control-enter submits the composer form", () => {
    const form = document.createElement("form");
    form.innerHTML = `<textarea></textarea><button type="submit">Send</button>`;
    let submitted = false;
    form.requestSubmit = () => {
        submitted = true;
    };
    const destroy = initComposer(form);
    form.querySelector("textarea")!.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true }),
    );
    expect(submitted).toBe(true);
    destroy();
});

test("plain enter does not submit", () => {
    const form = document.createElement("form");
    form.innerHTML = `<textarea></textarea>`;
    let submitted = false;
    form.requestSubmit = () => {
        submitted = true;
    };
    initComposer(form);
    form.querySelector("textarea")!.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter" }),
    );
    expect(submitted).toBe(false);
});
