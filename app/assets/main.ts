import "@fontsource/ibm-plex-sans/latin-400.css";
import "@fontsource/ibm-plex-sans/latin-600.css";
import "@fontsource/ibm-plex-sans/latin-700.css";
import "@fontsource/ibm-plex-mono/latin-400.css";
import "@fontsource/ibm-plex-mono/latin-500.css";
import "./input.css";
import { startApp } from "./hypergraft-bootstrap";
import { listenForRequestSettled } from "hypergraft/browser";

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

function selectConversationSettingsTab(panel: HTMLElement, id: string) {
    panel
        .querySelectorAll<HTMLElement>('[role="tabpanel"]')
        .forEach((section) => {
            section.hidden = section.id !== id;
        });
    panel.querySelectorAll<HTMLElement>('[role="tab"]').forEach((tab) => {
        const active = tab.getAttribute("aria-controls") === id;
        tab.setAttribute("aria-selected", String(active));
        tab.tabIndex = active ? 0 : -1;
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
    const save = panel.querySelector<HTMLElement>("[data-settings-save]");
    if (save)
        save.hidden = ["settings-details", "settings-presets"].includes(id);
}

function revealConversationSetting(target: HTMLElement, focus = true) {
    const panel = target.closest<HTMLElement>("#conversation-settings");
    const scroll = panel?.querySelector<HTMLElement>("[data-settings-scroll]");
    if (!panel || !scroll) return;
    const section = target.closest<HTMLElement>('[role="tabpanel"]');
    if (target.closest("[data-execution-actions]"))
        selectConversationSettingsTab(panel, "settings-execution");
    else if (section || target.id === "conversation-settings-heading")
        selectConversationSettingsTab(panel, section?.id ?? "settings-model");
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

document.addEventListener("click", (event) => {
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
    // Select before native activation so the first visible frame uses the destination tab.
    selectConversationSettingsTab(
        panel,
        target.closest('[role="tabpanel"]')?.id ?? "settings-model",
    );
    // Native activation retains the trigger for Escape focus restoration.
    requestAnimationFrame(() => {
        revealConversationSetting(
            target,
            shortcut.getAttribute("role") !== "tab",
        );
    });
});

document.addEventListener("keydown", (event) => {
    const tab = event.target;
    if (
        !(tab instanceof HTMLElement) ||
        !tab.matches('#conversation-settings [role="tab"]')
    )
        return;
    const tabs = Array.from(
        tab.parentElement!.querySelectorAll<HTMLElement>('[role="tab"]'),
    );
    const index = tabs.indexOf(tab);
    let next: number;
    switch (event.key) {
        case "ArrowRight":
        case "ArrowDown":
            next = (index + 1) % tabs.length;
            break;
        case "ArrowLeft":
        case "ArrowUp":
            next = (index + tabs.length - 1) % tabs.length;
            break;
        case "Home":
            next = 0;
            break;
        case "End":
            next = tabs.length - 1;
            break;
        default:
            return;
    }
    event.preventDefault();
    tabs[next].focus();
    tabs[next].click();
});

let settingsScroll: number | undefined;
let settingsTab: string | undefined;
document.addEventListener(
    "submit",
    () => {
        const panel = document.querySelector<HTMLElement>(
            "#conversation-settings:popover-open",
        );
        settingsScroll = panel?.querySelector<HTMLElement>(
            "[data-settings-scroll]",
        )?.scrollTop;
        settingsTab =
            panel
                ?.querySelector<HTMLElement>(
                    '[role="tab"][aria-selected="true"]',
                )
                ?.getAttribute("aria-controls") ?? undefined;
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
    const previousTab = settingsTab;
    settingsScroll = undefined;
    settingsTab = undefined;
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
        '#conversation-settings[data-settings-open="true"], #conversation-documents[data-documents-open="true"]',
    );
    // A top-layer panel must not conceal command or transport errors.
    document
        .querySelectorAll<HTMLElement>(".conversation-panel:popover-open")
        .forEach((panel) => {
            if (detail.outcome !== "applied-patch" || panel !== requestedPanel)
                panel.hidePopover();
        });
    if (detail.outcome === "applied-patch" && requestedPanel) {
        if (previousTab)
            selectConversationSettingsTab(requestedPanel, previousTab);
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
            selectConversationSettingsTab(panel, field.value);
            const scroll = panel.querySelector<HTMLElement>(
                "[data-settings-scroll]",
            );
            if (scroll) scroll.scrollTop = 0;
        }
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
