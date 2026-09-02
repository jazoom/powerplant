import "@fontsource/ibm-plex-sans/latin-400.css";
import "@fontsource/ibm-plex-sans/latin-600.css";
import "@fontsource/ibm-plex-sans/latin-700.css";
import "@fontsource/ibm-plex-mono/latin-400.css";
import "@fontsource/ibm-plex-mono/latin-500.css";
import "./input.css";
import { startApp } from "./hypergraft-bootstrap";
import { applyStoredTheme } from "./theme";

applyStoredTheme();
startApp();

const LIVE_RELOAD_EVENT_STREAM = "/_tower-livereload/event-stream";

function enableLiveReload() {
    window.addEventListener("pageshow", () => {
        const source = new EventSource(LIVE_RELOAD_EVENT_STREAM);

        source.addEventListener("reload", () => {
            source.close();
            window.location.reload();
        });

        const reloadWhenServerReturns = () => {
            source.removeEventListener("error", reloadWhenServerReturns);
            source.addEventListener("init", () => {
                source.close();
                window.location.reload();
            });
        };

        source.addEventListener("error", reloadWhenServerReturns);
        window.addEventListener("pagehide", () => {
            source.removeEventListener("error", reloadWhenServerReturns);
            source.close();
        });
    });
}

if (import.meta.env.MODE === "development") {
    enableLiveReload();
}
