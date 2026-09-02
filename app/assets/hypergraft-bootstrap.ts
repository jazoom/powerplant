import {
    bindTransportFeedback,
    listenForDiagnostics,
    startHypergraft,
} from "hypergraft/browser";
import { initComposer, initShortcutHint } from "./composer";
import { initConnectErrors } from "./connect-errors";
import { initConnectPlan } from "./connect-plan";
import { initDeskSettings } from "./desk-settings";
import { initObserve } from "./observe";
import { initThemeSelector } from "./theme";
import { initTranscript } from "./transcript";

export function startApp(): void {
    const bound = bindTransportFeedback(document);

    if (import.meta.env.DEV) {
        listenForDiagnostics((detail) => {
            console.error("Hypergraft diagnostic", detail.reason);
        });
    }

    startHypergraft({
        feedback: bound.feedback,
        islands: {
            composer: initComposer,
            "connect-errors": initConnectErrors,
            "connect-plan": initConnectPlan,
            "desk-settings": initDeskSettings,
            observe: initObserve,
            "shortcut-hint": initShortcutHint,
            "theme-selector": initThemeSelector,
            transcript: initTranscript,
        },
    });
}
