import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import zh from "./locales/zh.json";
import en from "./locales/en.json";
import vi from "./locales/vi.json";

const savedLanguage = localStorage.getItem("codex_plus_language") || "zh";

i18n
  .use(initReactI18next)
  .init({
    resources: {
      zh: { translation: zh },
      en: { translation: en },
      vi: { translation: vi },
    },
    lng: savedLanguage,
    fallbackLng: "zh",
    interpolation: {
      escapeValue: false,
    },
  });

i18n.on("languageChanged", (lng) => {
  localStorage.setItem("codex_plus_language", lng);
});

export default i18n;
