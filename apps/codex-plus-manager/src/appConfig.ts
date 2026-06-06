export const HELPER_HOST = "127.0.0.1";
export const DEFAULT_DEBUG_PORT = 9229;
export const DEFAULT_HELPER_PORT = 57321;
export const HELPER_API_PATH = "/v1";
export const PROTOCOL_PROXY_BASE_URL = `http://${HELPER_HOST}:${DEFAULT_HELPER_PORT}${HELPER_API_PATH}`;
export const PROTOCOL_PROXY_ENDPOINT = `${HELPER_HOST}:${DEFAULT_HELPER_PORT}`;
export const CHAT_UPSTREAM_BASE_URL_KEY = "codex_plus_chat_base_url";
export const APP_LINKS = {
    projectRepo: "https://github.com/BigPizzaV3/CodexPlusPlus",
    projectIssues: "https://github.com/BigPizzaV3/CodexPlusPlus/issues",
    discord: "https://discord.gg/y96kX7A76v",
    telegram: "https://t.me/CodexPlusPlus",
    scriptMarket: "https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket",
    adListRepo: "BigPizzaV3/Ad-List",
} as const;
export const PROJECT_REPO_DISPLAY = "github.com/BigPizzaV3/CodexPlusPlus";
export const CODEX_HOME_DIR = "~/.codex";
export const CODEX_SESSIONS_DB_FILE = "state_5.sqlite";
export const DEFAULT_CODEX_SESSIONS_DB_PATH = `${CODEX_HOME_DIR}/${CODEX_SESSIONS_DB_FILE}`;
export const DEFAULT_CODEX_AUTH_PATH = `${CODEX_HOME_DIR}/auth.json`;
export const DEFAULT_RELAY_PROFILE_ID = "default";
export const DEFAULT_RELAY_TEST_MODEL = "gpt-5.4-mini";
export const DEFAULT_CLI_WRAPPER_API_KEY_ENV = "CUSTOM_OPENAI_API_KEY";
export const STORAGE_KEYS = {
    lang: "codex-plus-lang",
    theme: "codex-plus-theme",
} as const;
export const TOAST_AUTO_CLOSE_MS = 4200;
export const DEFAULT_LOG_LINE_COUNT = 240;
export const PROVIDER_SYNC_PROGRESS = {
    initialPercent: 12,
    maxPercent: 88,
    stepPercent: 8,
    markerCheckThreshold: 40,
    tickMs: 350,
} as const;
