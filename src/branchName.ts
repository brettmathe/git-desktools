// Branch names the user types for a Smart Feature. Mirrors git's ref-name rules
// (`git check-ref-format --branch`); the backend re-checks with git itself.

export const BRANCH_NAME_RE =
  /^(?!-)(?!@$)(?!HEAD$)(?!\/)(?!.*\/$)(?!.*\/\/)(?!.*\.\.)(?!.*@\{)(?!.*\.$)(?!(?:.*\/)?\.)(?!.*\.lock(?:\/|$))[^\x00-\x20\x7f~^:?*[\\]+$/;

/** Each rule of BRANCH_NAME_RE on its own, so the message names what's wrong. */
const RULES: [RegExp, string][] = [
  [/^-/, "Can't start with -"],
  [/^@$/, "@ alone is reserved"],
  [/^HEAD$/, "HEAD is reserved"],
  [/^\//, "Can't start with /"],
  [/\/$/, "Can't end with /"],
  [/\/\//, "Can't contain //"],
  [/\.\./, "Can't contain .."],
  [/@\{/, "Can't contain @{"],
  [/\.$/, "Can't end with ."],
  [/(?:^|\/)\./, "No part can start with . (after a / or at the start)"],
  [/\.lock(?:\/|$)/, "No part can end with .lock"],
  [/[\x00-\x20\x7f]/, "Branch names can't contain spaces or control characters"],
  [/[~^:?*[\\]/, "Branch names can't contain ~ ^ : ? * [ or \\"],
];

/** null when `name` is a legal branch name, else why it isn't. */
export function branchNameError(name: string): string | null {
  if (!name) return "Type a branch name";
  if (BRANCH_NAME_RE.test(name)) return null;
  for (const [re, message] of RULES) if (re.test(name)) return message;
  return "Not a valid branch name";
}
