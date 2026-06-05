import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import { STORAGE_KEYS } from "../appConfig";
import en from "./en.json";
import zh from "./zh.json";

const savedLang = typeof window !== "undefined" ? window.localStorage.getItem(STORAGE_KEYS.lang) : null;

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    zh: { translation: zh },
  },
  lng: savedLang || "zh",
  fallbackLng: "zh",
  interpolation: {
    escapeValue: false,
  },
});

export default i18n;
