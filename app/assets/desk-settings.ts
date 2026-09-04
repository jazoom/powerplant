import type { IslandInstance, IslandMountContext } from "hypergraft/browser";

type DeskControls = {
    provider: HTMLSelectElement;
    model: HTMLInputElement;
    providerModelSynced: HTMLInputElement;
    thinking: HTMLSelectElement;
    combobox: HTMLElement;
    toggle: HTMLButtonElement;
    options: HTMLElement;
    search: HTMLInputElement;
    label: HTMLElement;
    noResults: HTMLElement;
};

function controls(root: HTMLFormElement): DeskControls | null {
    const provider = root.elements.namedItem("provider");
    const model = root.elements.namedItem("model");
    const providerModelSynced = root.elements.namedItem(
        "provider_model_synced",
    );
    const thinking = root.elements.namedItem("thinking");
    const combobox = root.querySelector<HTMLElement>("[data-model-combobox]");
    const toggle = root.querySelector<HTMLButtonElement>("[data-model-toggle]");
    const options = root.querySelector<HTMLElement>("[data-model-options]");
    const search = root.querySelector<HTMLInputElement>("[data-model-search]");
    const label = root.querySelector<HTMLElement>("[data-model-label]");
    const noResults = root.querySelector<HTMLElement>(
        "[data-model-no-results]",
    );
    if (
        !(provider instanceof HTMLSelectElement) ||
        !(model instanceof HTMLInputElement) ||
        !(providerModelSynced instanceof HTMLInputElement) ||
        !(thinking instanceof HTMLSelectElement) ||
        !combobox ||
        !toggle ||
        !options ||
        !search ||
        !label ||
        !noResults
    ) {
        return null;
    }
    return {
        provider,
        model,
        providerModelSynced,
        thinking,
        combobox,
        toggle,
        options,
        search,
        label,
        noResults,
    };
}

function modelOptions(found: DeskControls): HTMLButtonElement[] {
    return Array.from(
        found.options.querySelectorAll<HTMLButtonElement>("[data-model-value]"),
    );
}

function visibleModelOptions(found: DeskControls): HTMLButtonElement[] {
    return modelOptions(found).filter(
        (option) => !option.closest<HTMLElement>("[data-model-row]")?.hidden,
    );
}

function filterModels(found: DeskControls): void {
    const query = found.search.value.trim().toLocaleLowerCase();
    let visible = 0;
    for (const option of modelOptions(found)) {
        const row = option.closest<HTMLElement>("[data-model-row]");
        if (!row) {
            continue;
        }
        const matches =
            query === "" ||
            (option.dataset.modelValue ?? "")
                .toLocaleLowerCase()
                .includes(query);
        row.hidden = !matches;
        if (matches) {
            visible += 1;
        }
    }
    for (const group of found.options.querySelectorAll<HTMLElement>(
        "[data-model-group]",
    )) {
        group.hidden = !Array.from(
            group.querySelectorAll<HTMLElement>("[data-model-row]"),
        ).some((row) => !row.hidden);
    }
    found.noResults.hidden = query === "" || visible !== 0;
}

function syncModelSelection(root: HTMLFormElement): void {
    const found = controls(root);
    if (!found) {
        return;
    }
    const value = found.model.value.trim();
    found.label.textContent = value;
    for (const option of modelOptions(found)) {
        const selected = option.dataset.modelValue === value;
        option.ariaPressed = String(selected);
        option.classList.toggle("bg-base-300", selected);
    }
}

function setExpanded(found: DeskControls, expanded: boolean): void {
    found.combobox.classList.toggle("dropdown-open", expanded);
    found.options.hidden = !expanded;
    found.toggle.ariaExpanded = String(expanded);
}

function syncProvider(root: HTMLFormElement): void {
    const found = controls(root);
    if (!found) {
        return;
    }
    const selected = found.provider.selectedOptions.item(0);
    found.model.value = selected?.dataset.model ?? "";
    found.thinking.value = selected?.dataset.thinking ?? "default";
    found.providerModelSynced.value = "true";
    syncModelSelection(root);
    setExpanded(found, false);
}

function focusRelativeOption(
    found: DeskControls,
    current: HTMLButtonElement,
    offset: number,
): void {
    const options = visibleModelOptions(found);
    const currentIndex = options.indexOf(current);
    if (currentIndex < 0 || options.length === 0) {
        return;
    }
    options[(currentIndex + offset + options.length) % options.length]?.focus();
}

export function initDeskSettings(
    root: HTMLElement,
    { signal }: IslandMountContext,
): IslandInstance | void {
    if (!(root instanceof HTMLFormElement)) {
        return;
    }
    const submitDesk = () => {
        root.requestSubmit();
    };
    const selectListedModel = (
        found: DeskControls,
        option: HTMLButtonElement,
    ) => {
        found.model.value = option.dataset.modelValue ?? "";
        syncModelSelection(root);
        setExpanded(found, false);
        submitDesk();
    };
    const listedModelForEnter = (
        found: DeskControls,
    ): HTMLButtonElement | undefined => {
        const visible = visibleModelOptions(found);
        const query = found.search.value.trim();
        const exact = visible.find(
            (option) => (option.dataset.modelValue ?? "") === query,
        );
        if (exact) {
            return exact;
        }
        if (visible.length === 1) {
            return visible[0];
        }
        return undefined;
    };
    const focusSearch = (found: DeskControls) => {
        queueMicrotask(() => {
            if (!signal.aborted && found.toggle.ariaExpanded === "true") {
                found.search.focus();
            }
        });
    };
    root.addEventListener(
        "click",
        (event) => {
            const found = controls(root);
            if (!found || !(event.target instanceof Element)) {
                return;
            }
            if (event.target.closest("[data-model-toggle]")) {
                event.preventDefault();
                event.stopPropagation();
                const expanded = found.toggle.ariaExpanded !== "true";
                setExpanded(found, expanded);
                if (expanded) {
                    found.search.value = "";
                    filterModels(found);
                    focusSearch(found);
                }
                return;
            }
            if (event.target.closest("[data-model-favourite]")) {
                found.search.focus();
                return;
            }
            const option =
                event.target.closest<HTMLButtonElement>("[data-model-value]");
            if (!option) {
                return;
            }
            selectListedModel(found, option);
        },
        { signal },
    );
    root.addEventListener(
        "change",
        (event) => {
            if (!(event.target instanceof HTMLSelectElement)) {
                return;
            }
            if (event.target.name === "provider") {
                syncProvider(root);
                submitDesk();
                return;
            }
            if (event.target.name === "thinking") {
                submitDesk();
            }
        },
        { signal },
    );
    root.addEventListener(
        "input",
        (event) => {
            if (
                event.target instanceof HTMLInputElement &&
                event.target.matches("[data-model-search]")
            ) {
                const found = controls(root);
                if (found) {
                    filterModels(found);
                }
            }
        },
        { signal },
    );
    root.addEventListener(
        "keydown",
        (event) => {
            const found = controls(root);
            if (!found || !(event.target instanceof Element)) {
                return;
            }
            if (
                event.key === "Escape" &&
                event.target.closest("[data-model-options]")
            ) {
                event.preventDefault();
                setExpanded(found, false);
                found.toggle.focus();
                return;
            }
            if (event.target === found.search) {
                if (event.key === "ArrowDown") {
                    event.preventDefault();
                    visibleModelOptions(found)[0]?.focus();
                } else if (event.key === "Enter") {
                    event.preventDefault();
                    const option = listedModelForEnter(found);
                    if (option) {
                        selectListedModel(found, option);
                    }
                }
                return;
            }
            const option =
                event.target.closest<HTMLButtonElement>("[data-model-value]") ??
                event.target
                    .closest("[data-model-row]")
                    ?.querySelector("[data-model-value]");
            if (!option) {
                return;
            }
            if (event.key === "ArrowDown") {
                event.preventDefault();
                focusRelativeOption(found, option, 1);
            } else if (event.key === "ArrowUp") {
                event.preventDefault();
                focusRelativeOption(found, option, -1);
            } else if (event.key === "Home") {
                event.preventDefault();
                visibleModelOptions(found)[0]?.focus();
            } else if (event.key === "End") {
                event.preventDefault();
                visibleModelOptions(found).at(-1)?.focus();
            }
        },
        { signal },
    );
    root.addEventListener(
        "mousedown",
        (event) => {
            if (
                event.target instanceof Element &&
                event.target.closest("[data-model-favourite]")
            ) {
                event.preventDefault();
            }
        },
        { signal },
    );
    const closeOutside = (event: Event) => {
        const found = controls(root);
        if (
            found &&
            event.target instanceof Element &&
            !event.target.closest("[data-model-combobox]")
        ) {
            setExpanded(found, false);
        }
    };
    document.addEventListener("click", closeOutside, { signal });
    document.addEventListener("focusin", closeOutside, { signal });

    return {
        reconcile(context) {
            const targets = (() => {
                if (context.cause === "live-patch") {
                    return context.detail.targetIds;
                }
                if (
                    context.cause === "patch" &&
                    context.detail.outcome === "applied-patch"
                ) {
                    return context.detail.targetIds;
                }
                return undefined;
            })();
            if (!targets) {
                return;
            }
            if (
                targets.includes("desk-model-catalogue") ||
                targets.includes("desk-model-options")
            ) {
                syncModelSelection(root);
                const found = controls(root);
                if (found) {
                    filterModels(found);
                }
            }
        },
        destroy() {},
    };
}
