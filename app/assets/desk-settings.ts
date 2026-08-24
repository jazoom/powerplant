import { requestGraftRefresh } from "hypergraft/browser";
import type { IslandInstance, IslandMountContext } from "hypergraft/browser";

type DeskControls = {
    provider: HTMLSelectElement;
    model: HTMLInputElement;
    favourite: HTMLButtonElement;
    providerModelSynced: HTMLInputElement;
    combobox: HTMLElement;
    toggle: HTMLButtonElement;
    options: HTMLElement;
};

function controls(root: HTMLFormElement): DeskControls | null {
    const provider = root.elements.namedItem("provider");
    const model = root.elements.namedItem("model");
    const favourite = root.elements.namedItem("favourite");
    const providerModelSynced = root.elements.namedItem(
        "provider_model_synced",
    );
    const combobox = root.querySelector<HTMLElement>("[data-model-combobox]");
    const toggle = root.querySelector<HTMLButtonElement>("[data-model-toggle]");
    const options = root.querySelector<HTMLElement>("[data-model-options]");
    if (
        !(provider instanceof HTMLSelectElement) ||
        !(model instanceof HTMLInputElement) ||
        !(favourite instanceof HTMLButtonElement) ||
        !(providerModelSynced instanceof HTMLInputElement) ||
        !combobox ||
        !toggle ||
        !options
    ) {
        return null;
    }
    return {
        provider,
        model,
        favourite,
        providerModelSynced,
        combobox,
        toggle,
        options,
    };
}

function modelOptions(found: DeskControls): HTMLButtonElement[] {
    return Array.from(
        found.options.querySelectorAll<HTMLButtonElement>("[data-model-value]"),
    );
}

function setFavourite(favourite: HTMLButtonElement, pressed: boolean): void {
    favourite.ariaPressed = String(pressed);
    favourite.ariaLabel = pressed ? "Unfavourite model" : "Favourite model";
    for (const icon of favourite.querySelectorAll<HTMLElement>(
        "[data-favourite-icon]",
    )) {
        icon.hidden = icon.dataset.favouriteIcon !== (pressed ? "on" : "off");
    }
}

function syncFavourite(root: HTMLFormElement): void {
    const found = controls(root);
    if (!found) {
        return;
    }
    const value = found.model.value.trim();
    let favourite = false;
    for (const option of modelOptions(found)) {
        const selected = option.dataset.modelValue === value;
        option.ariaSelected = String(selected);
        option.classList.toggle("bg-base-300", selected);
        if (selected) {
            favourite = option.dataset.favourite === "true";
        }
    }
    setFavourite(found.favourite, favourite);
}

function setExpanded(found: DeskControls, expanded: boolean): void {
    found.combobox.classList.toggle("dropdown-open", expanded);
    found.options.hidden = !expanded;
    found.model.ariaExpanded = String(expanded);
    found.toggle.ariaExpanded = String(expanded);
}

function syncProvider(root: HTMLFormElement): void {
    const found = controls(root);
    if (!found) {
        return;
    }
    const selected = found.provider.selectedOptions.item(0);
    found.model.value = selected?.dataset.model ?? "";
    found.providerModelSynced.value = "true";
    setFavourite(
        found.favourite,
        selected?.dataset.currentFavourite === "true",
    );
    setExpanded(found, false);
}

function catalogueIsPending(root: HTMLElement): boolean {
    return (
        root.querySelector<HTMLElement>("[data-catalogue-pending]")?.dataset
            .cataloguePending === "true"
    );
}

function focusRelativeOption(
    found: DeskControls,
    current: HTMLButtonElement,
    offset: number,
): void {
    const options = modelOptions(found);
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
    let refreshTimer: number | undefined;
    const submitDesk = () => {
        window.clearTimeout(refreshTimer);
        root.requestSubmit();
    };
    const scheduleCatalogueRefresh = () => {
        if (!catalogueIsPending(root) || signal.aborted) {
            return;
        }
        window.clearTimeout(refreshTimer);
        refreshTimer = window.setTimeout(() => {
            const found = controls(root);
            if (found?.options.contains(document.activeElement)) {
                scheduleCatalogueRefresh();
                return;
            }
            const form = document.querySelector<HTMLFormElement>(
                "#desk-model-refresh",
            );
            if (form) {
                requestGraftRefresh(form);
            }
        }, 250);
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
                setExpanded(found, found.toggle.ariaExpanded !== "true");
                return;
            }
            const option =
                event.target.closest<HTMLButtonElement>("[data-model-value]");
            if (!option) {
                return;
            }
            found.model.value = option.dataset.modelValue ?? "";
            syncFavourite(root);
            setExpanded(found, false);
            submitDesk();
        },
        { signal },
    );
    root.addEventListener(
        "change",
        (event) => {
            if (
                event.target instanceof HTMLSelectElement &&
                event.target.name === "provider"
            ) {
                syncProvider(root);
                submitDesk();
                return;
            }
            if (
                event.target instanceof HTMLInputElement &&
                event.target.name === "model"
            ) {
                const found = controls(root);
                if (found) {
                    setExpanded(found, false);
                }
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
                event.target.name === "model"
            ) {
                syncFavourite(root);
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
            if (event.target === found.model) {
                if (event.key === "ArrowDown") {
                    event.preventDefault();
                    setExpanded(found, true);
                    const options = modelOptions(found);
                    const selected = options.find(
                        (option) => option.ariaSelected === "true",
                    );
                    (selected ?? options[0])?.focus();
                } else if (event.key === "Escape") {
                    setExpanded(found, false);
                } else if (event.key === "Enter") {
                    event.preventDefault();
                    setExpanded(found, false);
                    submitDesk();
                }
                return;
            }
            const option =
                event.target.closest<HTMLButtonElement>("[data-model-value]");
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
                modelOptions(found)[0]?.focus();
            } else if (event.key === "End") {
                event.preventDefault();
                modelOptions(found).at(-1)?.focus();
            } else if (event.key === "Escape") {
                event.preventDefault();
                setExpanded(found, false);
                found.model.focus();
            }
        },
        { signal },
    );
    const closeOutside = (event: Event) => {
        const found = controls(root);
        if (
            found &&
            event.target instanceof Node &&
            !root.contains(event.target)
        ) {
            setExpanded(found, false);
        }
    };
    document.addEventListener("click", closeOutside, { signal });
    document.addEventListener("focusin", closeOutside, { signal });
    signal.addEventListener(
        "abort",
        () => {
            window.clearTimeout(refreshTimer);
        },
        { once: true },
    );
    scheduleCatalogueRefresh();

    return {
        reconcile(context) {
            if (
                context.cause !== "patch" ||
                context.detail.outcome !== "applied-patch"
            ) {
                return;
            }
            const targets = context.detail.targetIds;
            if (targets.includes("desk-model-options")) {
                syncFavourite(root);
            }
            if (
                targets.includes("desk-model-options") ||
                targets.includes("desk-settings")
            ) {
                scheduleCatalogueRefresh();
            }
        },
        destroy() {
            window.clearTimeout(refreshTimer);
        },
    };
}
