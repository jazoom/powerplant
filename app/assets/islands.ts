import { observeIslands } from "hypergraft/browser/islands";
import { initComposer } from "./composer";
import { initTranscript } from "./transcript";

const islands = {
    composer: initComposer,
    transcript: initTranscript,
};

if (document.readyState === "loading")
    document.addEventListener(
        "DOMContentLoaded",
        () => observeIslands(islands),
        { once: true },
    );
else observeIslands(islands);
