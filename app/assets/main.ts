import "@fontsource/ibm-plex-sans/latin-400.css";
import "@fontsource/ibm-plex-sans/latin-600.css";
import "@fontsource/ibm-plex-sans/latin-700.css";
import "@fontsource/ibm-plex-mono/latin-400.css";
import "@fontsource/ibm-plex-mono/latin-500.css";
import "./input.css";
import { startApp } from "./hypergraft-bootstrap";
import { listenForRequestSettled } from "hypergraft/browser";

type ReviewCatalogueModel = {
    id: string;
    default_effort: string;
    efforts: { value: string; label: string }[];
};

// Independent review pages offer the conversation model catalogue through
// constrained selects. Provider and model changes refresh the dependent
// options from the embedded catalogue. The server still validates every
// choice, so native submission works without this enhancement.
function reviewCatalogue(
    section: HTMLElement,
): Record<string, ReviewCatalogueModel[]> {
    const source = section.parentElement?.querySelector<HTMLElement>(
        "[data-review-model-catalogue]",
    );
    const raw = source?.dataset.reviewModelCatalogue ?? "{}";
    try {
        const parsed: unknown = JSON.parse(raw);
        if (parsed && typeof parsed === "object" && !Array.isArray(parsed))
            return parsed as Record<string, ReviewCatalogueModel[]>;
    } catch {
        // A malformed catalogue keeps the server-rendered options.
    }
    return {};
}

function syncReviewEfforts(
    section: HTMLElement,
    catalogue: Record<string, ReviewCatalogueModel[]>,
) {
    const provider = section.querySelector<HTMLSelectElement>(
        "[data-review-provider]",
    );
    const model = section.querySelector<HTMLSelectElement>(
        "[data-review-model]",
    );
    const thinking = section.querySelector<HTMLSelectElement>(
        "[data-review-thinking]",
    );
    if (!provider || !model || !thinking) return;
    const models = catalogue[provider.value] ?? [];
    const selected = models.find((item) => item.id === model.value);
    const efforts = selected?.efforts ?? [];
    const previous = thinking.value;
    thinking.replaceChildren(
        ...(efforts.length
            ? efforts.map((effort) => new Option(effort.label, effort.value))
            : [new Option("Not available", "")]),
    );
    thinking.value = efforts.some((effort) => effort.value === previous)
        ? previous
        : (selected?.default_effort ?? "");
}

function syncReviewModels(section: HTMLElement, resetModel: boolean) {
    const provider = section.querySelector<HTMLSelectElement>(
        "[data-review-provider]",
    );
    const model = section.querySelector<HTMLSelectElement>(
        "[data-review-model]",
    );
    if (!provider || !model) return;
    const catalogue = reviewCatalogue(section);
    const models = catalogue[provider.value] ?? [];
    let selected = model.value;
    if (resetModel && !models.some((item) => item.id === selected))
        selected = models[0]?.id ?? "";
    // A provider change replaces the model list. The server-rendered
    // Unavailable entry never survives a move to another provider.
    if (models.length === 0) {
        if (selected) {
            model.replaceChildren(
                new Option(`Unavailable · ${selected}`, selected),
            );
            model.value = selected;
        } else {
            const empty = new Option("No models available", "");
            empty.disabled = true;
            model.replaceChildren(empty);
            model.value = "";
        }
    } else {
        model.replaceChildren(
            ...models.map((item) => new Option(item.id, item.id)),
        );
        model.value = models.some((item) => item.id === selected)
            ? selected
            : (models[0]?.id ?? "");
    }
    syncReviewEfforts(section, catalogue);
}

function syncExecutionModeFields() {
    const location = document.querySelector<HTMLInputElement>(
        'input[form="conversation-settings-form"][name="location"]:checked',
    );
    if (!location) return;
    const host = location.value === "host";
    document
        .querySelectorAll<HTMLElement>("[data-execution-sandbox-settings]")
        .forEach((section) => {
            section.hidden = host;
        });
    const policy = document.querySelector<HTMLElement>(
        "[data-execution-host-policy]",
    );
    if (policy) policy.hidden = !host;
}

function selectConversationSettingsSection(panel: HTMLElement, id: string) {
    panel
        .querySelectorAll<HTMLElement>("[data-settings-panel]")
        .forEach((section) => {
            section.hidden = section.id !== id;
        });
    const menu = panel.querySelector<HTMLSelectElement>("[data-settings-menu]");
    if (menu) menu.value = id;
    const actions = panel.querySelector<HTMLElement>(
        "[data-execution-actions]",
    );
    if (actions)
        actions.hidden = ![
            "settings-execution",
            "settings-directories",
        ].includes(id);
    const footer = panel.querySelector<HTMLElement>("[data-settings-footer]");
    if (footer)
        footer.hidden = ["settings-details", "settings-presets"].includes(id);
}

function revealConversationSetting(target: HTMLElement, focus = true) {
    const panel = target.closest<HTMLElement>("#conversation-settings");
    const scroll = panel?.querySelector<HTMLElement>("[data-settings-scroll]");
    if (!panel || !scroll) return;
    const section = target.closest<HTMLElement>("[data-settings-panel]");
    if (target.closest("[data-execution-actions]"))
        selectConversationSettingsSection(panel, "settings-execution");
    else if (section || target.id === "conversation-settings-heading")
        selectConversationSettingsSection(
            panel,
            section?.id ?? "settings-model",
        );
    let parent = target.parentElement;
    while (parent && parent !== panel) {
        if (parent instanceof HTMLDetailsElement) parent.open = true;
        parent = parent.parentElement;
    }
    if (!panel.matches(":popover-open")) panel.showPopover();
    if (focus) target.focus({ preventScroll: true });
    scroll.scrollTop =
        scroll.contains(target) && !target.classList.contains("sr-only")
            ? scroll.scrollTop +
              target.getBoundingClientRect().top -
              scroll.getBoundingClientRect().top -
              16
            : 0;
}

// The empty transcript example fills the composer only. It submits nothing
// and creates no record or directory access. The prompt stays editable.
const EXAMPLE_PROMPT =
    "Make connection errors more helpful. Keep my message if a request fails, and give me a clear next step.";

document.addEventListener("click", (event) => {
    const example =
        event.target instanceof Element
            ? event.target.closest<HTMLElement>("[data-example-prompt]")
            : null;
    if (example) {
        const message = document.getElementById("composer-message");
        if (message instanceof HTMLTextAreaElement) {
            message.value = EXAMPLE_PROMPT;
            message.dispatchEvent(new Event("input", { bubbles: true }));
            message.focus();
        }
        return;
    }
    const shortcut =
        event.target instanceof Element
            ? event.target.closest<HTMLElement>("[data-settings-section]")
            : null;
    if (!shortcut) return;
    let target = document.getElementById(
        shortcut.dataset.settingsSection ?? "",
    );
    if (target?.closest("[data-execution-sandbox-settings][hidden]"))
        target = document.getElementById("conversation-execution-heading");
    if (!target) return;
    const panel = target.closest<HTMLElement>("#conversation-settings");
    if (!panel) return;
    selectConversationSettingsSection(
        panel,
        target.closest("[data-settings-panel]")?.id ?? "settings-model",
    );
    // Native activation retains the trigger for Escape focus restoration.
    requestAnimationFrame(() => revealConversationSetting(target));
});

let settingsScroll: number | undefined;
let settingsSection: string | undefined;
document.addEventListener(
    "submit",
    () => {
        const panel = document.querySelector<HTMLElement>(
            "#conversation-settings:popover-open",
        );
        settingsScroll = panel?.querySelector<HTMLElement>(
            "[data-settings-scroll]",
        )?.scrollTop;
        settingsSection = panel?.querySelector<HTMLSelectElement>(
            "[data-settings-menu]",
        )?.value;
    },
    true,
);

listenForRequestSettled((detail) => {
    if (
        detail.outcome === "applied-patch" &&
        !detail.targetIds.includes("conversation-detail")
    ) {
        return;
    }
    const previousScroll = settingsScroll;
    const previousSection = settingsSection;
    settingsScroll = undefined;
    settingsSection = undefined;
    syncExecutionModeFields();
    // Retained external submitters can retain transport-only ARIA state after a patch.
    document
        .querySelectorAll<HTMLButtonElement>(
            "#conversation-settings button[form]",
        )
        .forEach((button) => {
            if (
                !button.disabled &&
                !button.hasAttribute("data-graft-submitter-pending")
            )
                button.removeAttribute("aria-disabled");
        });
    const requestedPanel = document.querySelector<HTMLElement>(
        '#conversation-settings[data-settings-open="true"]',
    );
    // A top-layer panel must not conceal command or transport errors.
    document
        .querySelectorAll<HTMLElement>(".conversation-panel:popover-open")
        .forEach((panel) => {
            if (detail.outcome !== "applied-patch" || panel !== requestedPanel)
                panel.hidePopover();
        });
    if (detail.outcome === "applied-patch" && requestedPanel) {
        if (previousSection)
            selectConversationSettingsSection(requestedPanel, previousSection);
        if (!requestedPanel.matches(":popover-open"))
            requestedPanel.showPopover();
        const destination =
            requestedPanel.querySelector<HTMLElement>(
                "[data-execution-switch]:not([hidden]), [data-preset-preview], #conversation-directory-consent",
            ) ??
            (requestedPanel.dataset.directoriesOpen === "true"
                ? document.getElementById("conversation-directory-heading")
                : null);
        const scroll = requestedPanel.querySelector<HTMLElement>(
            "[data-settings-scroll]",
        );
        if (destination) revealConversationSetting(destination);
        else if (scroll && previousScroll !== undefined)
            scroll.scrollTop = previousScroll;
    }
    if (detail.outcome === "applied-patch" && detail.status !== 200) {
        (
            requestedPanel?.querySelector<HTMLElement>('[role="alert"]') ??
            document.querySelector<HTMLElement>("#conversation-error") ??
            requestedPanel
        )?.focus();
    }
});

document.addEventListener("change", (event) => {
    const field = event.target;
    if (
        field instanceof HTMLSelectElement &&
        field.matches("[data-settings-menu]")
    ) {
        const panel = field.closest<HTMLElement>("#conversation-settings");
        if (panel) {
            selectConversationSettingsSection(panel, field.value);
            const scroll = panel.querySelector<HTMLElement>(
                "[data-settings-scroll]",
            );
            if (scroll) scroll.scrollTop = 0;
        }
    }
    if (
        field instanceof HTMLSelectElement &&
        field.matches("[data-review-provider]")
    ) {
        const section = field.closest<HTMLElement>(
            "[data-review-model-picker]",
        );
        if (section) syncReviewModels(section, true);
    }
    if (
        field instanceof HTMLSelectElement &&
        field.matches("[data-review-model]")
    ) {
        const section = field.closest<HTMLElement>(
            "[data-review-model-picker]",
        );
        if (section) syncReviewEfforts(section, reviewCatalogue(section));
    }
    if (
        field instanceof HTMLSelectElement &&
        field.matches("[data-draft-directory-access]")
    ) {
        const submitter = document.getElementById(
            field.dataset.draftDirectoryAccess ?? "",
        );
        if (submitter instanceof HTMLButtonElement && submitter.form) {
            submitter.value = field.value;
            submitter.form.requestSubmit(submitter);
        }
    }
    if (
        field instanceof HTMLSelectElement &&
        field.id === "conversation-environment"
    ) {
        document
            .querySelectorAll<HTMLElement>("[data-environment-problem]")
            .forEach((problem) => {
                problem.hidden =
                    problem.dataset.environmentProblem !== field.value;
            });
        const status = document.querySelector<HTMLElement>(
            "[data-conversation-environment-status]",
        );
        if (
            status &&
            document.querySelector('[data-conversation-state="new"]')
        ) {
            const label = field.selectedOptions[0]?.dataset.environmentStatus;
            status.textContent = label ?? "";
            status.setAttribute("aria-label", `Status: ${label ?? ""}`);
            status.hidden = !label;
        }
    }
    if (
        field instanceof HTMLInputElement &&
        field.matches("[data-workflow-location]")
    ) {
        const preview = field
            .closest("fieldset")
            ?.querySelector<HTMLButtonElement>(
                "[data-workflow-location-preview]",
            );
        if (preview && field.form) field.form.requestSubmit(preview);
    }
    if (
        field instanceof HTMLInputElement &&
        field.matches("[data-workflow-execution-mode]")
    ) {
        const preview = field.form?.querySelector<HTMLButtonElement>(
            "[data-workflow-mode-preview]",
        );
        if (preview && field.form) field.form.requestSubmit(preview);
    }
    if (
        field instanceof HTMLInputElement &&
        field.form?.id === "conversation-composer" &&
        (field.name === "location" ||
            field.name === "tool_run" ||
            field.name === "host_approval")
    ) {
        const preview = document.querySelector<HTMLButtonElement>(
            "[data-location-preview]",
        );
        if (preview?.form === field.form) field.form.requestSubmit(preview);
    }
    if (
        field instanceof HTMLInputElement &&
        field.name === "location" &&
        field.form?.id === "conversation-settings-form"
    ) {
        syncExecutionModeFields();
    }
    if (
        field instanceof HTMLSelectElement &&
        field.matches("[data-execution-directory]")
    ) {
        const value = document.querySelector<HTMLInputElement>(
            "#execution-directory-access",
        );
        if (value) {
            value.value = JSON.stringify(
                Array.from(
                    document.querySelectorAll<HTMLSelectElement>(
                        "[data-execution-directory]",
                    ),
                    (select) => [
                        select.dataset.executionDirectory,
                        select.value,
                    ],
                ),
            );
            value.dispatchEvent(new Event("input", { bubbles: true }));
        }
    }
    if (
        (event.target instanceof HTMLSelectElement &&
            (event.target.id === "conversation-environment" ||
                event.target.matches("[data-execution-directory]"))) ||
        (event.target instanceof HTMLInputElement &&
            (event.target.name === "location" ||
                event.target.name === "host_approval"))
    ) {
        document
            .querySelector<HTMLElement>("[data-execution-switch]")
            ?.setAttribute("hidden", "");
    }
});

document.addEventListener(
    "invalid",
    (event) => {
        if (!(event.target instanceof HTMLElement)) return;
        if (event.target.closest("#conversation-settings")) {
            revealConversationSetting(event.target);
            return;
        }
        if (!event.target.closest("#workflow-launch")) return;
        let parent = event.target.parentElement;
        while (parent && parent.id !== "workflow-launch") {
            parent.hidden = false;
            if (parent instanceof HTMLDetailsElement) parent.open = true;
            parent = parent.parentElement;
        }
    },
    true,
);

startApp();

const LIVE_RELOAD_EVENT_STREAM = "/_tower-livereload/event-stream";
const LIVE_RELOAD_CHANNEL = "powerplant-live-reload";

// One event stream per tab can exhaust the browser HTTP/1.1 connection pool.
// Keep the stream in the visible tab. Use BroadcastChannel to notify hidden tabs to reload.
function enableLiveReload() {
    window.addEventListener("pageshow", () => {
        let source: EventSource | null = null;
        let reloading = false;
        const channel = new BroadcastChannel(LIVE_RELOAD_CHANNEL);

        const closeStream = () => {
            if (!source) return;
            source.close();
            source = null;
        };

        const reload = (broadcast: boolean) => {
            if (reloading) return;
            reloading = true;
            closeStream();
            if (broadcast) channel.postMessage(null);
            channel.close();
            window.location.reload();
        };

        channel.addEventListener("message", () => reload(false));

        const openStream = () => {
            if (source || document.visibilityState !== "visible") return;
            const next = new EventSource(LIVE_RELOAD_EVENT_STREAM);
            source = next;

            next.addEventListener("reload", () => reload(true));

            const reloadWhenServerReturns = () => {
                next.removeEventListener("error", reloadWhenServerReturns);
                next.addEventListener("init", () => reload(true));
            };

            next.addEventListener("error", reloadWhenServerReturns);
        };

        const onVisibility = () => {
            if (document.visibilityState === "visible") {
                openStream();
            } else {
                closeStream();
            }
        };

        document.addEventListener("visibilitychange", onVisibility);
        window.addEventListener("pagehide", () => {
            document.removeEventListener("visibilitychange", onVisibility);
            closeStream();
            channel.close();
        });
        openStream();
    });
}

if (import.meta.env.MODE === "development") {
    enableLiveReload();
}
