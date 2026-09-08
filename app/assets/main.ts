import "@fontsource/ibm-plex-sans/latin-400.css";
import "@fontsource/ibm-plex-sans/latin-600.css";
import "@fontsource/ibm-plex-sans/latin-700.css";
import "@fontsource/ibm-plex-mono/latin-400.css";
import "@fontsource/ibm-plex-mono/latin-500.css";
import "./input.css";
import { startApp } from "./hypergraft-bootstrap";
import { listenForRequestSettled } from "hypergraft/browser";

listenForRequestSettled((detail) => {
    if (
        detail.outcome === "applied-patch" &&
        !detail.targetIds.includes("conversation-detail")
    ) {
        return;
    }
    const requestedPanel = document.querySelector<HTMLElement>(
        '.conversation-panel[data-settings-open="true"], .conversation-panel[data-directories-open="true"]',
    );
    // A top-layer panel must not conceal command or transport errors.
    document
        .querySelectorAll<HTMLElement>(".conversation-panel:popover-open")
        .forEach((panel) => {
            if (detail.outcome !== "applied-patch" || panel !== requestedPanel)
                panel.hidePopover();
        });
    if (
        detail.outcome === "applied-patch" &&
        requestedPanel &&
        !requestedPanel.matches(":popover-open")
    )
        requestedPanel.showPopover();
    if (detail.outcome === "applied-patch" && detail.status !== 200) {
        (
            document.querySelector<HTMLElement>("#conversation-error") ??
            requestedPanel
        )?.focus();
    }
});

document.addEventListener("change", (event) => {
    const field = event.target;
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
        event.target instanceof HTMLSelectElement &&
        event.target.id === "conversation-environment"
    ) {
        document
            .querySelector<HTMLElement>("[data-environment-switch]")
            ?.setAttribute("hidden", "");
    }
});

document.addEventListener(
    "invalid",
    (event) => {
        if (
            !(event.target instanceof HTMLElement) ||
            !event.target.closest("#workflow-launch")
        )
            return;
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
