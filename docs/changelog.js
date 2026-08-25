const changelogRepository = "BigPizzaV3/CodexPlusPlus";

const escapeHtml = (value) => String(value || "").replace(/[&<>"']/g, (character) => ({
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
}[character]));

const inlineMarkdown = (value) => escapeHtml(value)
  .replace(/`([^`]+)`/g, "<code>$1</code>")
  .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
  .replace(/\[([^\]]+)\]\((https:\/\/[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1 ↗</a>');

const markdownToHtml = (markdown) => {
  const output = [];
  let listOpen = false;
  const closeList = () => {
    if (!listOpen) return;
    output.push("</ul>");
    listOpen = false;
  };

  String(markdown || "").split(/\r?\n/).forEach((line) => {
    const trimmed = line.trim();
    if (!trimmed) {
      closeList();
      return;
    }
    const heading = trimmed.match(/^#{1,3}\s+(.+)$/);
    if (heading) {
      closeList();
      output.push(`<h3>${inlineMarkdown(heading[1])}</h3>`);
      return;
    }
    const bullet = trimmed.match(/^[-*+]\s+(.+)$/);
    if (bullet) {
      if (!listOpen) {
        output.push("<ul>");
        listOpen = true;
      }
      output.push(`<li>${inlineMarkdown(bullet[1])}</li>`);
      return;
    }
    closeList();
    output.push(`<p>${inlineMarkdown(trimmed)}</p>`);
  });
  closeList();
  return output.join("") || "<p>—</p>";
};

const releaseDate = (release) => {
  const value = release.published_at || release.created_at;
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return `${date.getFullYear()}.${String(date.getMonth() + 1).padStart(2, "0")}.${String(date.getDate()).padStart(2, "0")}`;
};

const releaseId = (release, index) => {
  const tag = String(release.tag_name || `release-${index + 1}`)
    .toLowerCase()
    .replace(/[^a-z0-9-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return `release-${tag || index + 1}`;
};

const releaseTitle = (release) => release.name || release.tag_name || "Codex++ release";

const renderRelease = (release, index) => {
  const tag = escapeHtml(release.tag_name || release.name || `release-${index + 1}`);
  const date = releaseDate(release);
  const id = releaseId(release, index);
  const name = escapeHtml(releaseTitle(release));
  const summary = escapeHtml(release.name && release.name !== release.tag_name ? release.name : "");
  const badge = index === 0
    ? '<span class="changelog-badge"><span class="lang lang-zh">最新</span><span class="lang lang-en">Latest</span></span>'
    : "";
  return `<article class="changelog-entry" id="${id}">
    <header class="changelog-entry-header">
      <div>
        <p class="changelog-version">${tag} ${badge}</p>
        <p class="changelog-date">${escapeHtml(date)}</p>
      </div>
      <a class="changelog-entry-link" href="${escapeHtml(release.html_url || `https://github.com/${changelogRepository}/releases`)}" target="_blank" rel="noreferrer" aria-label="${tag} GitHub Release">↗</a>
    </header>
    ${summary ? `<p class="changelog-summary">${summary}</p>` : ""}
    <section class="changelog-section changelog-release-body">
      <h2><span class="lang lang-zh">发布说明</span><span class="lang lang-en">Release notes</span></h2>
      ${markdownToHtml(release.body)}
    </section>
  </article>`;
};

const renderReleaseNav = (releases) => releases.map((release, index) => {
  const tag = escapeHtml(release.tag_name || release.name || `Release ${index + 1}`);
  return `<a class="${index === 0 ? "is-current" : ""}" href="#${releaseId(release, index)}"${index === 0 ? ' aria-current="location"' : ""}><strong>${tag}</strong><span>${escapeHtml(releaseDate(release))}</span></a>`;
}).join("");

const setupReleaseNavigation = () => {
  const navigation = document.querySelector("[data-release-nav]");
  if (!navigation) return;

  const setCurrent = (link) => {
    navigation.querySelectorAll("a").forEach((item) => {
      const current = item === link;
      item.classList.toggle("is-current", current);
      if (current) item.setAttribute("aria-current", "location");
      else item.removeAttribute("aria-current");
    });
  };

  navigation.addEventListener("click", (event) => {
    const link = event.target.closest("a");
    if (link) setCurrent(link);
  });

  const hash = window.location.hash;
  if (hash) {
    const link = navigation.querySelector(`a[href="${CSS.escape(hash)}"]`);
    if (link) setCurrent(link);
  }
};

const showStatus = (message, error = false) => {
  const status = document.querySelector("[data-release-status]");
  if (!status) return;
  status.classList.toggle("is-error", error);
  if (message) {
    status.innerHTML = `<span class="lang lang-zh">${escapeHtml(message.zh)}</span><span class="lang lang-en">${escapeHtml(message.en)}</span>`;
  }
  status.hidden = !message;
};

const loadChangelog = async () => {
  const list = document.querySelector("[data-release-list]");
  const navigation = document.querySelector("[data-release-nav]");
  if (!list || !navigation) return;

  try {
    const response = await fetch(`https://api.github.com/repos/${changelogRepository}/releases?per_page=20`, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) throw new Error(`GitHub Releases API returned ${response.status}`);
    const releases = (await response.json()).filter((release) => !release.draft && release.tag_name);
    if (!releases.length) throw new Error("No releases returned");

    list.innerHTML = releases.map(renderRelease).join("");
    navigation.innerHTML = renderReleaseNav(releases);
    setupReleaseNavigation();
    showStatus(null);
  } catch (error) {
    showStatus({
      zh: "GitHub Releases 暂时不可用，当前显示内置记录。",
      en: "GitHub Releases is temporarily unavailable; showing the bundled records.",
    }, true);
    console.info("Using bundled changelog records", error);
  }
};

loadChangelog();
