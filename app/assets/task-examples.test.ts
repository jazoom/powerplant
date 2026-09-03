// @vitest-environment happy-dom
import { expect, test, vi } from "vitest";
import { initTaskExamples } from "./task-examples";

const STRUCTURE = "Explain how this project is structured.";
const REVIEW = "Review the current code and identify one concern.";
const IMPROVE = "Find one small improvement and make it.";

function mountDesk(messageDisabled = false): {
    island: HTMLElement;
    message: HTMLTextAreaElement;
    form: HTMLFormElement;
    requestSubmit: ReturnType<typeof vi.fn>;
} {
    document.body.innerHTML = `
        <div data-island="task-examples">
            <button type="button" data-task-example="${STRUCTURE}">${STRUCTURE}</button>
            <button type="button" data-task-example="${REVIEW}">${REVIEW}</button>
            <button type="button" data-task-example="${IMPROVE}">${IMPROVE}</button>
        </div>
        <form id="composer-send">
            <textarea id="composer-message" ${messageDisabled ? "disabled" : ""}></textarea>
            <button type="submit" name="mode" value="quick">Send</button>
        </form>
    `;
    const island = document.querySelector<HTMLElement>(
        "[data-island='task-examples']",
    )!;
    const form = document.querySelector<HTMLFormElement>("#composer-send")!;
    const requestSubmit = vi.fn();
    form.requestSubmit = requestSubmit;
    initTaskExamples(island, { signal: new AbortController().signal });
    return {
        island,
        message: document.querySelector("#composer-message")!,
        form,
        requestSubmit,
    };
}

test("an example fills the composer and moves focus", () => {
    const { island, message } = mountDesk();
    island.querySelectorAll("button")[0]!.click();
    expect(message.value).toBe(STRUCTURE);
    expect(document.activeElement).toBe(message);
});

test("an example does not submit a task", () => {
    const { island, form, requestSubmit } = mountDesk();
    const submit = vi.fn((event: Event) => event.preventDefault());
    form.addEventListener("submit", submit);
    island.querySelectorAll("button")[1]!.click();
    expect(requestSubmit).not.toHaveBeenCalled();
    expect(submit).not.toHaveBeenCalled();
});

test("an example does not change the composer disabled state", () => {
    const { island, message } = mountDesk(true);
    expect(message.disabled).toBe(true);
    island.querySelectorAll("button")[2]!.click();
    expect(message.value).toBe(IMPROVE);
    expect(message.disabled).toBe(true);
});
