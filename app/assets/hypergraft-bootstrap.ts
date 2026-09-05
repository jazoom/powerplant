import {
    bindTransportFeedback,
    listenForDiagnostics,
    startHypergraft,
} from "hypergraft/browser";
import { initAppContext } from "./app-context";
import { initComposer, initShortcutHint } from "./composer";
import { initConnectErrors } from "./connect-errors";
import { initConnectPlan } from "./connect-plan";
import { initDeskSettings } from "./desk-settings";
import { initNavigationMore } from "./navigation-more";
import { initObserve } from "./observe";
import { initTaskExamples } from "./task-examples";
import { initThemeSelector } from "./theme";
import { initThinkingVisibility } from "./thinking-visibility";
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
            "app-context": initAppContext,
            composer: initComposer,
            "connect-errors": initConnectErrors,
            "connect-plan": initConnectPlan,
            "desk-settings": initDeskSettings,
            "navigation-more": initNavigationMore,
            observe: initObserve,
            "shortcut-hint": initShortcutHint,
            "task-examples": initTaskExamples,
            "theme-selector": initThemeSelector,
            "thinking-visibility": initThinkingVisibility,
            transcript: initTranscript,
        },
    });
}
