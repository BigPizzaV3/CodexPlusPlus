const GITHUB_HOSTS = new Set(["github.com", "www.github.com"]);
const GITHUB_NON_OWNER_ROUTES = new Set([
  "about",
  "account",
  "advisories",
  "apps",
  "business",
  "codespaces",
  "collections",
  "contact",
  "copilot",
  "customer-stories",
  "dashboard",
  "edu",
  "enterprise",
  "events",
  "explore",
  "features",
  "git-guides",
  "github-copilot",
  "issues",
  "login",
  "marketplace",
  "mcp",
  "mobile",
  "new",
  "newsletter",
  "newsroom",
  "notifications",
  "open-source",
  "organizations",
  "orgs",
  "pricing",
  "pulls",
  "readme",
  "resources",
  "search",
  "security",
  "settings",
  "site",
  "sitemap",
  "social-impact",
  "solutions",
  "sponsors",
  "stars",
  "team",
  "topics",
  "trending",
  "users",
  "why-github",
]);
const GITHUB_OWNER = /^[A-Za-z0-9]+(?:-[A-Za-z0-9]+)*$/;
const GITHUB_REPOSITORY = /^[A-Za-z0-9._-]+$/;

export function isGitHubRepositoryHomepage(value: string): boolean {
  try {
    const url = new URL(value);
    if (url.protocol !== "https:" || !GITHUB_HOSTS.has(url.hostname)) return false;
    if (url.username || url.password || url.port) return false;

    const path = /^\/([^/]+)\/([^/]+)\/?$/.exec(url.pathname);
    if (!path) return false;

    const [, owner, rawRepository] = path;
    const repository = rawRepository.endsWith(".git") ? rawRepository.slice(0, -4) : rawRepository;
    return (
      owner.length <= 39 &&
      !GITHUB_NON_OWNER_ROUTES.has(owner.toLowerCase()) &&
      GITHUB_OWNER.test(owner) &&
      repository.length <= 100 &&
      GITHUB_REPOSITORY.test(repository)
    );
  } catch {
    return false;
  }
}
