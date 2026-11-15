// Self-hosted Monaco (no CDN — works offline and under a strict CSP).
import * as monaco from "monaco-editor";
import { loader } from "@monaco-editor/react";
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
import JsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker";
import CssWorker from "monaco-editor/esm/vs/language/css/css.worker?worker";
import HtmlWorker from "monaco-editor/esm/vs/language/html/html.worker?worker";
import TsWorker from "monaco-editor/esm/vs/language/typescript/ts.worker?worker";

(self as any).MonacoEnvironment = {
  getWorker(_: string, label: string) {
    if (label === "json") return new JsonWorker();
    if (["css", "scss", "less"].includes(label)) return new CssWorker();
    if (["html", "handlebars", "razor"].includes(label)) return new HtmlWorker();
    if (["typescript", "javascript"].includes(label)) return new TsWorker();
    return new EditorWorker();
  },
};

// nginx has no built-in grammar in Monaco; this Monarch tokenizer covers
// directives, blocks, variables, strings, regex locations and comments.
monaco.languages.register({ id: "nginx", aliases: ["Nginx", "nginx"] });
monaco.languages.setLanguageConfiguration("nginx", {
  comments: { lineComment: "#" },
  brackets: [["{", "}"]],
  autoClosingPairs: [
    { open: "{", close: "}" },
    { open: '"', close: '"' },
    { open: "'", close: "'" },
  ],
});
monaco.languages.setMonarchTokensProvider("nginx", {
  blockKeywords: ["http", "server", "location", "upstream", "events", "stream", "map", "geo", "if", "types", "limit_except"],
  tokenizer: {
    root: [
      [/#.*$/, "comment"],
      [/\$[A-Za-z_][\w]*/, "variable"],
      [/"([^"\\]|\\.)*"/, "string"],
      [/'([^'\\]|\\.)*'/, "string"],
      [/[{}]/, "delimiter.bracket"],
      [/;/, "delimiter"],
      [/~\*?|\^~|=/, "operator"],
      [/\b\d+[kKmMgGsShHdD]?\b/, "number"],
      [/\b(on|off)\b/, "keyword.constant"],
      [/^\s*[a-z_][a-z0-9_]*/, { cases: { "@blockKeywords": "keyword.block", "@default": "keyword" } }],
      [/[^\s;{}#"']+/, "identifier"],
    ],
  },
} as any);

monaco.editor.defineTheme("orbital-dark", {
  base: "vs-dark",
  inherit: true,
  rules: [
    { token: "comment", foreground: "6E7A91", fontStyle: "italic" },
    { token: "keyword", foreground: "8FB4FF" },
    { token: "keyword.block", foreground: "F2B544", fontStyle: "bold" },
    { token: "keyword.constant", foreground: "C79BFF" },
    { token: "variable", foreground: "7FD1B9" },
    { token: "string", foreground: "B5D88A" },
    { token: "number", foreground: "F29E6D" },
  ],
  colors: {
    "editor.background": "#1D2534",
    "editor.lineHighlightBackground": "#242E40",
    "editorLineNumber.foreground": "#4A5670",
    "editorLineNumber.activeForeground": "#A9B3C7",
    "editor.selectionBackground": "#F2B54433",
    "editorCursor.foreground": "#F2B544",
    "editorIndentGuide.background1": "#2A3448",
    "editorWidget.background": "#222B3D",
    "diffEditor.insertedTextBackground": "#4FBF8B22",
    "diffEditor.removedTextBackground": "#E5645A22",
  },
});

monaco.editor.defineTheme("orbital-light", {
  base: "vs",
  inherit: true,
  rules: [
    { token: "comment", foreground: "7A859A", fontStyle: "italic" },
    { token: "keyword", foreground: "2F5FB3" },
    { token: "keyword.block", foreground: "A8700A", fontStyle: "bold" },
    { token: "keyword.constant", foreground: "8A4FC7" },
    { token: "variable", foreground: "1B8A7A" },
    { token: "string", foreground: "3F7F1F" },
    { token: "number", foreground: "C0562B" },
  ],
  colors: {
    "editor.background": "#FFFFFF",
    "editor.lineHighlightBackground": "#F3F5F8",
    "editorLineNumber.foreground": "#A9B3C3",
    "editorLineNumber.activeForeground": "#4B566C",
    "editor.selectionBackground": "#E3A32C40",
    "editorCursor.foreground": "#C98A12",
    "editorIndentGuide.background1": "#E3E8EF",
    "editorWidget.background": "#F3F5F8",
    "diffEditor.insertedTextBackground": "#23905F22",
    "diffEditor.removedTextBackground": "#CC423722",
  },
});

loader.config({ monaco });
export { monaco };
