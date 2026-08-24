// @vitest-environment happy-dom
import { afterEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ requestGraftRefresh: vi.fn() }));
vi.mock("hypergraft/browser", () => ({
    requestGraftRefresh: mocks.requestGraftRefresh,
}));

import { initDeskSettings } from "./desk-settings";

afterEach(() => {
    mocks.requestGraftRefresh.mockClear();
    vi.useRealTimers();
    document.body.replaceChildren();
});

function desk(): HTMLFormElement {
    const form = document.createElement("form");
    form.innerHTML = `
        <input type="hidden" name="provider_model_synced" value="false">
        <select name="provider">
            <option value="synthetic" data-model="hf:moonshotai/Kimi-K3" data-current-favourite="false" selected>Synthetic</option>
            <option value="openai-codex" data-model="gpt-5.1-codex" data-current-favourite="true">OpenAI</option>
        </select>
        <div data-model-combobox>
            <input name="model" value="hf:moonshotai/Kimi-K3" maxlength="256" role="combobox" aria-expanded="false">
            <button type="button" data-model-toggle aria-expanded="false">Show models</button>
            <div data-model-options role="listbox" hidden>
                <div data-catalogue-pending="false">
                    <button type="button" role="option" data-model-value="hf:moonshotai/Kimi-K3" data-favourite="false" aria-selected="true">Kimi</button>
                    <button type="button" role="option" data-model-value="syn:large:text" data-favourite="true" aria-selected="false">Large</button>
                    <button type="button" role="option" data-model-value="syn:small:text" data-favourite="false" aria-selected="false">Small</button>
                </div>
            </div>
        </div>
        <button name="favourite" type="submit" aria-pressed="false" aria-label="Favourite model">
            <span data-favourite-icon="on" hidden></span>
            <span data-favourite-icon="off"></span>
        </button>
    `;
    document.body.append(form);
    return form;
}

function mount(form: HTMLFormElement) {
    const controller = new AbortController();
    const island = initDeskSettings(form, { signal: controller.signal });
    const submit = vi.spyOn(form, "requestSubmit").mockImplementation(() => {});
    return { controller, island, submit };
}

test("the model button opens the full list despite the current input value", () => {
    const form = desk();
    const { controller } = mount(form);
    const toggle = form.querySelector<HTMLButtonElement>(
        "[data-model-toggle]",
    )!;
    const options = form.querySelector<HTMLElement>("[data-model-options]")!;

    toggle.click();

    expect(options.hidden).toBe(false);
    expect(
        Array.from(
            options.querySelectorAll<HTMLElement>("[data-model-value]"),
            (option) => option.dataset.modelValue,
        ),
    ).toEqual(["hf:moonshotai/Kimi-K3", "syn:large:text", "syn:small:text"]);
    controller.abort();
});

test("a listed model selection updates and submits the editable field", () => {
    const form = desk();
    const { controller, submit } = mount(form);
    const model = form.elements.namedItem("model") as HTMLInputElement;
    const favourite = form.elements.namedItem("favourite") as HTMLButtonElement;
    const option = form.querySelector<HTMLButtonElement>(
        '[data-model-value="syn:large:text"]',
    )!;

    option.click();

    expect(model.value).toBe("syn:large:text");
    expect(favourite.ariaPressed).toBe("true");
    expect(favourite.ariaLabel).toBe("Unfavourite model");
    expect(submit).toHaveBeenCalledOnce();
    controller.abort();
});

test("a custom model submits after a change", () => {
    const form = desk();
    const { controller, submit } = mount(form);
    const model = form.elements.namedItem("model") as HTMLInputElement;
    const favourite = form.elements.namedItem("favourite") as HTMLButtonElement;

    model.value = "hf:local/custom-model";
    model.dispatchEvent(new Event("input", { bubbles: true }));
    model.dispatchEvent(new Event("change", { bubbles: true }));

    expect(favourite.ariaPressed).toBe("false");
    expect(submit).toHaveBeenCalledOnce();
    controller.abort();
});

test("a provider change selects and submits that provider's saved model", () => {
    const form = desk();
    const { controller, submit } = mount(form);
    const provider = form.elements.namedItem("provider") as HTMLSelectElement;
    const model = form.elements.namedItem("model") as HTMLInputElement;
    const favourite = form.elements.namedItem("favourite") as HTMLButtonElement;
    const synced = form.elements.namedItem(
        "provider_model_synced",
    ) as HTMLInputElement;

    provider.value = "openai-codex";
    provider.dispatchEvent(new Event("change", { bubbles: true }));

    expect(model.value).toBe("gpt-5.1-codex");
    expect(favourite.ariaPressed).toBe("true");
    expect(synced.value).toBe("true");
    expect(submit).toHaveBeenCalledOnce();
    controller.abort();
});

test("focus in the model list waits before a catalogue refresh", () => {
    vi.useFakeTimers();
    const form = desk();
    form.querySelector<HTMLElement>(
        "[data-catalogue-pending]",
    )!.dataset.cataloguePending = "true";
    const refresh = document.createElement("form");
    refresh.id = "desk-model-refresh";
    document.body.append(refresh);
    const { controller } = mount(form);
    const option = form.querySelector<HTMLButtonElement>(
        '[data-model-value="syn:large:text"]',
    )!;

    option.focus();
    vi.advanceTimersByTime(1000);

    expect(mocks.requestGraftRefresh).not.toHaveBeenCalled();
    controller.abort();
});

test("a pending catalogue requests another refresh after a pending patch", () => {
    vi.useFakeTimers();
    const form = desk();
    form.querySelector<HTMLElement>(
        "[data-catalogue-pending]",
    )!.dataset.cataloguePending = "true";
    const refresh = document.createElement("form");
    refresh.id = "desk-model-refresh";
    document.body.append(refresh);
    const { controller, island } = mount(form);

    vi.advanceTimersByTime(250);
    island?.reconcile?.({
        cause: "patch",
        detail: {
            outcome: "applied-patch",
            targetIds: ["desk-model-options"],
        },
    } as never);
    vi.advanceTimersByTime(250);

    expect(mocks.requestGraftRefresh).toHaveBeenCalledTimes(2);
    controller.abort();
});
