const ISSUE_QUERY_PARAM = "issue";

export function readIssueIdentifier(search: string): string | null {
  const identifier = new URLSearchParams(search).get(ISSUE_QUERY_PARAM)?.trim().toUpperCase();
  return identifier || null;
}

export function buildIssueUrl(
  href: string,
  projectId: string | null,
  issueIdentifier: string | null,
): URL {
  const url = new URL(href);

  if (projectId) url.searchParams.set("project", projectId);
  else url.searchParams.delete("project");

  if (issueIdentifier) url.searchParams.set(ISSUE_QUERY_PARAM, issueIdentifier.trim().toUpperCase());
  else url.searchParams.delete(ISSUE_QUERY_PARAM);

  return url;
}
