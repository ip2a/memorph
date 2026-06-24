export function createI18n(getLanguage) {
  let dictionaries = { zh: {}, en: {} };

  function lang() {
    return getLanguage() || "zh";
  }

  function t(key) {
    return dictionaries[lang()]?.[key] || dictionaries.zh[key] || key;
  }

  function setDocumentLanguage() {
    document.documentElement.lang = lang() === "zh" ? "zh-CN" : "en";
  }

  async function loadI18n() {
    const response = await fetch("/i18n.json", {
      headers: {
        Accept: "application/json",
      },
    });
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}`);
    }
    dictionaries = await response.json();
    setDocumentLanguage();
  }

  return {
    lang,
    t,
    loadI18n,
    setDocumentLanguage,
  };
}
