import "@fontsource/ibm-plex-sans/latin-400.css";
import "@fontsource/ibm-plex-sans/latin-600.css";
import "@fontsource/ibm-plex-sans/latin-700.css";
import "@fontsource/ibm-plex-mono/latin-400.css";
import "@fontsource/ibm-plex-mono/latin-500.css";
import "./input.css";
import { startApp } from "./hypergraft-bootstrap";

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
