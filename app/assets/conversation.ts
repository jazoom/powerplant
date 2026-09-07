import {
    commandBlockReason,
    listenForRequestSettled,
    type IslandInstance,
    type IslandMountContext,
} from "hypergraft/browser";

export function initConversation(
    root: HTMLElement,
    { signal }: IslandMountContext,
): IslandInstance {
    function modelForm() {
        return root.querySelector<HTMLInputElement>("#conversation-model")
            ?.form;
    }

    const choices = new Map<string, { model: string; thinking: string }>();
    let pendingPreference = false;
    let deferredSend:
        { form: HTMLFormElement; submitter: HTMLElement | null } | undefined;
    function sendStatus(message: string) {
        const status = root.querySelector("#conversation-send-status");
        if (status) status.textContent = message;
    }
    function rememberModel() {
        const preference = root.querySelector<HTMLFormElement>(
            "#conversation-model-preference",
        );
        const source = modelForm();
        if (!preference || !source) return;
        const provider = source.elements.namedItem(
            "provider",
        ) as HTMLSelectElement;
        choices.set(provider.value, {
            model: (source.elements.namedItem("model") as HTMLInputElement)
                .value,
            thinking: (
                source.elements.namedItem("thinking") as HTMLSelectElement
            ).value,
        });
        pendingPreference = true;
        if (commandBlockReason()) return;
        for (const name of ["provider", "model", "thinking"]) {
            const field = source.elements.namedItem(name) as
                HTMLInputElement | HTMLSelectElement;
            (preference.elements.namedItem(name) as HTMLInputElement).value =
                field.disabled ? "" : field.value;
        }
        pendingPreference = false;
        preference.requestSubmit();
    }
    root.addEventListener(
        "submit",
        (event) => {
            if (
                event.defaultPrevented ||
                !(event.target instanceof HTMLFormElement) ||
                event.target.id !== "conversation-composer" ||
                root
                    .querySelector("#conversation-model-preference")
                    ?.getAttribute("aria-busy") !== "true"
            )
                return;
            // Only defer an unsent message behind this page's preference request.
            event.preventDefault();
            deferredSend ??= { form: event.target, submitter: event.submitter };
            sendStatus(
                "Your message will send after the model preference request finishes.",
            );
        },
        { signal },
    );
    const stopSettlement = listenForRequestSettled((detail) => {
        if (
            deferredSend &&
            detail.form === root.querySelector("#conversation-model-preference")
        ) {
            const { form, submitter } = deferredSend;
            deferredSend = undefined;
            pendingPreference = false;
            if (detail.outcome !== "applied-patch") {
                sendStatus(
                    "The message was not sent. The page needs a reload before another command.",
                );
                return;
            }
            sendStatus("");
            if (
                !commandBlockReason() &&
                root.contains(form) &&
                form.isConnected &&
                (!submitter ||
                    ((submitter instanceof HTMLButtonElement ||
                        submitter instanceof HTMLInputElement) &&
                        submitter.form === form &&
                        !submitter.disabled))
            )
                form.requestSubmit(submitter ?? undefined);
            return;
        }
        if (detail.outcome === "applied-patch" && pendingPreference)
            rememberModel();
    });

    function syncConversation() {
        // Saved summaries describe the authoritative configuration, not unsubmitted choices.
        if (!root.querySelector('[data-conversation-state="new"]')) return;
        const form = root.querySelector<HTMLFormElement>(
            "#conversation-composer",
        );
        if (!form) return;
        const field = (name: string) =>
            form.elements.namedItem(name) as
                HTMLInputElement | HTMLSelectElement | null;
        const preset = field("preset") as HTMLSelectElement | null;
        const model = root.querySelector("[data-conversation-model-summary]");
        if (model) {
            model.textContent = preset?.value
                ? `Preset: ${preset.selectedOptions[0].text}`
                : field("model")?.value || "Choose a model";
        }
        const project = field("project") as HTMLSelectElement | null;
        const context = root.querySelector<HTMLElement>(
            "[data-conversation-project-summary]",
        );
        if (context && project) {
            context.hidden = !project.value;
            context.textContent = project.value
                ? `${project.selectedOptions[0].text} · No file access`
                : "";
        }
    }

    function updateModelOptions(changed: HTMLSelectElement | HTMLInputElement) {
        const form = modelForm();
        const source = root.querySelector<HTMLElement>(
            "[data-conversation-model-catalogue]",
        );
        if (!form || !source) return;
        type Model = {
            id: string;
            default_effort: string;
            efforts: { value: string; label: string }[];
        };
        const catalogue: Record<string, Model[]> = JSON.parse(
            source.dataset.conversationModelCatalogue ?? "{}",
        );
        const provider = form.elements.namedItem(
            "provider",
        ) as HTMLSelectElement;
        const model = form.elements.namedItem("model") as HTMLInputElement;
        const thinking = form.elements.namedItem(
            "thinking",
        ) as HTMLSelectElement;
        if (provider.disabled) return;
        const models = catalogue[provider.value] ?? [];
        const search = root.querySelector<HTMLInputElement>(
            "#conversation-model-search",
        );
        const status = root.querySelector("#conversation-model-search-status");
        if (changed.name === "provider") {
            if (search) search.value = "";
            setModelExpanded(false);
            const preferred =
                choices.get(provider.value)?.model ??
                provider.selectedOptions[0]?.dataset.defaultModel;
            model.value =
                models.find((item) => item.id === preferred)?.id ??
                models[0]?.id ??
                "";
        }
        const selected = models.find((item) => item.id === model.value);
        const query = search?.value.trim().toLocaleLowerCase() ?? "";
        const matches = models.filter((item) =>
            item.id.toLocaleLowerCase().includes(query),
        );
        root.querySelector("#conversation-model-results")?.replaceChildren(
            ...matches.map((item) => {
                const button = document.createElement("button");
                button.type = "button";
                button.dataset.conversationModelValue = item.id;
                button.textContent = item.id;
                button.className =
                    "btn btn-ghost h-auto min-h-11 w-full min-w-0 justify-start text-left break-all font-normal aria-pressed:bg-base-300";
                button.ariaPressed = String(item === selected);
                return button;
            }),
        );
        const toggle = root.querySelector<HTMLButtonElement>(
            "#conversation-model-toggle",
        );
        if (toggle) toggle.disabled = models.length === 0;
        const label = root.querySelector("#conversation-model-value");
        if (label) label.textContent = model.value || "No models available";
        if (status) {
            status.textContent = matches.length
                ? `${matches.length} ${matches.length === 1 ? "model" : "models"}.`
                : "No matching models. Clear the search to see all models.";
        }
        // Search changes only the results, never the submitted model or effort.
        if (changed === search) return;
        const efforts = selected?.efforts ?? [];
        thinking.replaceChildren(
            ...(efforts.length
                ? efforts.map(
                      (effort) => new Option(effort.label, effort.value),
                  )
                : [new Option("Not available", "")]),
        );
        thinking.disabled = efforts.length === 0;
        const remembered = choices.get(provider.value);
        thinking.value =
            remembered?.model === model.value &&
            efforts.some((effort) => effort.value === remembered.thinking)
                ? remembered.thinking
                : (selected?.default_effort ?? "");
    }

    function setModelExpanded(expanded: boolean, restoreFocus = false) {
        const toggle = root.querySelector<HTMLButtonElement>(
            "#conversation-model-toggle",
        );
        const options = root.querySelector<HTMLElement>(
            "#conversation-model-options",
        );
        const search = root.querySelector<HTMLInputElement>(
            "#conversation-model-search",
        );
        if (!toggle || !options || !search || (expanded && toggle.disabled))
            return;
        toggle.ariaExpanded = String(expanded);
        options.hidden = !expanded;
        if (expanded) {
            search.value = "";
            updateModelOptions(search);
            search.focus();
            options.scrollIntoView({ block: "nearest" });
        } else if (restoreFocus) {
            toggle.focus();
        }
    }

    root.addEventListener(
        "click",
        (event) => {
            if (!(event.target instanceof Element)) return;
            const toggle = event.target.closest<HTMLButtonElement>(
                "#conversation-model-toggle",
            );
            if (toggle) {
                setModelExpanded(toggle.ariaExpanded !== "true");
                return;
            }
            const option = event.target.closest<HTMLButtonElement>(
                "[data-conversation-model-value]",
            );
            const model = root.querySelector<HTMLInputElement>(
                "#conversation-model",
            );
            if (
                option &&
                model &&
                !root.querySelector<HTMLSelectElement>('[name="provider"]')
                    ?.disabled
            ) {
                if (model.value !== option.dataset.conversationModelValue) {
                    model.value = option.dataset.conversationModelValue ?? "";
                    updateModelOptions(model);
                    syncConversation();
                    rememberModel();
                }
                setModelExpanded(false, true);
            } else if (!event.target.closest("#conversation-model-picker")) {
                setModelExpanded(false);
            }
        },
        { signal },
    );

    root.addEventListener(
        "focusin",
        (event) => {
            if (
                event.target instanceof Element &&
                !event.target.closest("#conversation-model-picker")
            ) {
                setModelExpanded(false);
            }
        },
        { signal },
    );

    root.addEventListener(
        "keydown",
        (event) => {
            if (
                !(event.target instanceof Element) ||
                !event.target.closest("#conversation-model-picker")
            )
                return;
            const search = root.querySelector<HTMLInputElement>(
                "#conversation-model-search",
            );
            const toggle = root.querySelector<HTMLButtonElement>(
                "#conversation-model-toggle",
            );
            if (event.key === "Escape" && toggle?.ariaExpanded === "true") {
                event.preventDefault();
                event.stopPropagation();
                setModelExpanded(false, true);
                return;
            }
            if (
                event.target === toggle &&
                ["ArrowDown", "ArrowUp"].includes(event.key)
            ) {
                event.preventDefault();
                setModelExpanded(true);
                return;
            }
            const options = Array.from(
                root.querySelectorAll<HTMLButtonElement>(
                    "[data-conversation-model-value]",
                ),
            );
            if (event.target === search && event.key === "Enter") {
                event.preventDefault();
                const query = search.value.trim().toLocaleLowerCase();
                const exact = options.find(
                    (option) =>
                        option.dataset.conversationModelValue?.toLocaleLowerCase() ===
                        query,
                );
                (
                    exact ?? (options.length === 1 ? options[0] : undefined)
                )?.click();
                return;
            }
            const index = options.indexOf(event.target as HTMLButtonElement);
            if (event.target !== search && index < 0) return;
            let next: HTMLElement | undefined;
            if (event.key === "ArrowDown")
                next = options[(index + 1) % options.length];
            else if (event.key === "ArrowUp")
                next =
                    index === 0
                        ? (search ?? undefined)
                        : options.at(index < 0 ? -1 : index - 1);
            else if (event.target !== search && event.key === "Home")
                next = options[0];
            else if (event.target !== search && event.key === "End")
                next = options.at(-1);
            else return;
            event.preventDefault();
            next?.focus();
        },
        { signal },
    );

    root.addEventListener(
        "input",
        (event) => {
            if (
                event.target instanceof HTMLInputElement &&
                event.target.id === "conversation-model-search"
            ) {
                updateModelOptions(event.target);
                return;
            }
            if (
                event.target instanceof HTMLInputElement &&
                event.target.form?.id === "conversation-composer"
            ) {
                syncConversation();
            }
        },
        { signal },
    );

    root.addEventListener(
        "change",
        (event) => {
            if (event.target instanceof HTMLSelectElement) {
                if (
                    event.target.form === modelForm() &&
                    event.target.name === "provider"
                )
                    updateModelOptions(event.target);
                syncConversation();
                if (
                    event.target.form === modelForm() &&
                    ["provider", "thinking"].includes(event.target.name)
                )
                    rememberModel();
            }
        },
        { signal },
    );

    syncConversation();
    return {
        reconcile(context) {
            if (
                context.cause !== "location" &&
                (!("targetIds" in context.detail) ||
                    !context.detail.targetIds.some(
                        (id) => id === root.id || id === "chat-main",
                    ))
            )
                return;
            pendingPreference = false;
            deferredSend = undefined;
            choices.clear();
            setModelExpanded(false);
            const search = root.querySelector<HTMLInputElement>(
                "#conversation-model-search",
            );
            if (search) search.value = "";
            root.querySelector(
                "#conversation-model-results",
            )?.replaceChildren();
            syncConversation();
        },
        destroy() {
            stopSettlement();
        },
    };
}
