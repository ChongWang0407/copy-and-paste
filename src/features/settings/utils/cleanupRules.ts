/**
 * Cleanup rules as stored: one `pattern => replacement` line per rule, with
 * optional `# label:` / `# action:` comment lines above it (read by Rust's
 * parse_cleanup_rules).
 */
export interface EditableRule {
    match: string;
    replace: string;
    label?: string;
    action?: "clean" | "ignore";
}

// A rule is one line: line breaks typed in either box are stored as \n,
// which the regex and the replacement both understand.
const toOneLine = (text: string): string => text.replace(/\r\n|\r|\n/g, "\\n");

export const parseRules = (rawRules: string): EditableRule[] => {
    const lines = rawRules.split(/\r?\n/).map((line) => line.trim());
    const rules: EditableRule[] = [];
    let currentLabel: string | null = null;
    let currentAction: "clean" | "ignore" | undefined = undefined;

    for (const line of lines) {
        if (line.length === 0) continue;
        if (line.startsWith("#")) {
            const labelMatch = line.match(/^#\s*label:\s*(.*)$/i);
            if (labelMatch) {
                currentLabel = labelMatch[1].trim();
            }
            const actionMatch = line.match(/^#\s*action:\s*(.*)$/i);
            if (actionMatch) {
                const actValue = actionMatch[1].trim().toLowerCase();
                currentAction = actValue === "ignore" ? "ignore" : "clean";
            }
            continue;
        }

        // The first "=>" separates them, as in Rust; later ones belong to the
        // replacement (split(/=>/, 2) dropped them).
        const separator = line.indexOf("=>");
        const matchPart = separator === -1 ? line : line.slice(0, separator);
        const replacePart = separator === -1 ? "" : line.slice(separator + 2);
        rules.push({
            match: matchPart.trim(),
            replace: replacePart.trim(),
            label: currentLabel ?? undefined,
            action: currentAction ?? "clean"
        });
        currentLabel = null;
        currentAction = undefined;
    }

    return rules;
};

export const serializeRules = (rules: EditableRule[]): string =>
    rules
        .filter((rule) => rule.match.trim().length > 0)
        .map((rule) => {
            const lines = [];
            if (rule.label?.trim()) {
                lines.push(`# label: ${toOneLine(rule.label.trim())}`);
            }
            if (rule.action === "ignore") {
                lines.push("# action: ignore");
            }
            lines.push(`${toOneLine(rule.match.trim())} => ${toOneLine(rule.replace)}`);
            return lines.join("\n");
        })
        .join("\n\n");
