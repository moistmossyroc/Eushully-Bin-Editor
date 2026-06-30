// Help Center content index. Each section is a markdown file under `src/help/`,
// bundled as a raw string (Vite `?raw`) and rendered in HelpDialog. To add a
// section: drop a new `.md` file and add one entry here.
import gettingStarted from "../help/getting-started.md?raw";
import loading from "../help/loading.md?raw";
import characters from "../help/characters.md?raw";
import translating from "../help/translating.md?raw";
import ccTranslate from "../help/cc-translate.md?raw";
import glossary from "../help/glossary.md?raw";
import autoSplit from "../help/auto-split.md?raw";
import fanpatch from "../help/fanpatch.md?raw";
import bilingual from "../help/bilingual.md?raw";
import troubleshooting from "../help/troubleshooting.md?raw";

export interface HelpSection {
  /** Stable id used for deep-linking from contextual `?` buttons. */
  id: string;
  /** Nav label. */
  title: string;
  /** Raw markdown body. */
  markdown: string;
}

export const HELP_SECTIONS: HelpSection[] = [
  { id: "getting-started", title: "Getting Started", markdown: gettingStarted },
  { id: "loading", title: "Loading BINs & folders", markdown: loading },
  { id: "characters", title: "Characters & speakers", markdown: characters },
  { id: "translating", title: "Translating", markdown: translating },
  { id: "cc-translate", title: "CC Translate (offline)", markdown: ccTranslate },
  { id: "glossary", title: "Glossary", markdown: glossary },
  { id: "auto-split", title: "Auto-split & boxes", markdown: autoSplit },
  { id: "fanpatch", title: "Fanpatch & loader", markdown: fanpatch },
  { id: "bilingual", title: "Bilingual language swap", markdown: bilingual },
  { id: "troubleshooting", title: "Troubleshooting", markdown: troubleshooting },
];

export const DEFAULT_HELP_SECTION = "getting-started";
