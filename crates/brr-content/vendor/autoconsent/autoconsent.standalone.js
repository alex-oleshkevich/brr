"use strict";
(() => {
  var __typeError = (msg) => {
    throw TypeError(msg);
  };
  var __accessCheck = (obj, member, msg) => member.has(obj) || __typeError("Cannot " + msg);
  var __privateGet = (obj, member, getter) => (__accessCheck(obj, member, "read from private field"), getter ? getter.call(obj) : member.get(obj));
  var __privateAdd = (obj, member, value) => member.has(obj) ? __typeError("Cannot add the same private member more than once") : member instanceof WeakSet ? member.add(obj) : member.set(obj, value);
  var __privateSet = (obj, member, value, setter) => (__accessCheck(obj, member, "write to private field"), setter ? setter.call(obj, value) : member.set(obj, value), value);

  // lib/rules.ts
  var SUPPORTED_RULE_STEP_VERSION = 3;

  // lib/random.ts
  function getRandomID() {
    if (crypto && typeof crypto.randomUUID !== "undefined") {
      return crypto.randomUUID();
    }
    return Math.random().toString();
  }

  // lib/eval-handler.ts
  var Deferred = class {
    constructor(id, timeout = 1e3) {
      this.id = id;
      this.promise = new Promise((resolve, reject) => {
        this.resolve = resolve;
        this.reject = reject;
      });
      this.timer = window.setTimeout(() => {
        this.reject(new Error("timeout"));
      }, timeout);
    }
  };
  var evalState = {
    pending: /* @__PURE__ */ new Map(),
    sendContentMessage: null
  };
  function requestEval(code, snippetId) {
    if (!evalState.sendContentMessage) {
      return Promise.reject(new Error("AutoConsent is not initialized yet"));
    }
    const id = getRandomID();
    evalState.sendContentMessage({
      type: "eval",
      id,
      code,
      snippetId
    });
    const deferred = new Deferred(id);
    evalState.pending.set(deferred.id, deferred);
    return deferred.promise;
  }
  function resolveEval(id, value) {
    const deferred = evalState.pending.get(id);
    if (deferred) {
      evalState.pending.delete(id);
      deferred.timer && window.clearTimeout(deferred.timer);
      deferred.resolve(value);
    } else {
      console.warn("no eval #", id);
    }
  }

  // lib/utils.ts
  function getStyleElement(styleOverrideElementId = "autoconsent-css-rules") {
    const styleSelector = `style#${styleOverrideElementId}`;
    const existingElement = document.querySelector(styleSelector);
    if (existingElement && existingElement instanceof HTMLStyleElement) {
      return existingElement;
    } else {
      const parent = document.head || document.getElementsByTagName("head")[0] || document.documentElement;
      const css = document.createElement("style");
      css.id = styleOverrideElementId;
      parent.appendChild(css);
      return css;
    }
  }
  function getHidingStyle(method) {
    const hidingSnippet = method === "opacity" ? `opacity: 0` : `display: none`;
    return `${hidingSnippet} !important; z-index: -1 !important; pointer-events: none !important;`;
  }
  function hideElements(styleEl, selector, method = "display") {
    const rule = `${selector} { ${getHidingStyle(method)} } `;
    if (styleEl instanceof HTMLStyleElement) {
      styleEl.innerText += rule;
      return selector.length > 0;
    }
    return false;
  }
  function appendStylesheetRule(styleEl, cssRule, marker) {
    if (!(styleEl instanceof HTMLStyleElement) || cssRule.length === 0) {
      return false;
    }
    const markerComment = marker ? `/* autoconsent:${marker} */` : "";
    if (markerComment && styleEl.innerText.includes(markerComment)) {
      return true;
    }
    if (!markerComment && styleEl.innerText.includes(cssRule.trim())) {
      return true;
    }
    styleEl.innerText += `${markerComment}${markerComment ? " " : ""}${cssRule} `;
    return true;
  }
  async function waitFor(predicate, maxTimes, interval) {
    const result = await predicate();
    if (!result && maxTimes > 0) {
      return new Promise((resolve) => {
        setTimeout(async () => {
          resolve(waitFor(predicate, maxTimes - 1, interval));
        }, interval);
      });
    }
    return Promise.resolve(result);
  }
  function isElementVisible(elem) {
    if (!elem) {
      return false;
    }
    if (elem.offsetParent !== null) {
      return true;
    } else {
      const css = window.getComputedStyle(elem);
      if (css.position === "fixed" && css.display !== "none") {
        return true;
      }
    }
    return false;
  }
  function copyObject(data) {
    if (globalThis.structuredClone) {
      return structuredClone(data);
    }
    return JSON.parse(JSON.stringify(data));
  }
  function normalizeConfig(providedConfig) {
    const defaultConfig = {
      enabled: true,
      autoAction: "optOut",
      // if falsy, the extension will wait for an explicit user signal before opting in/out
      disabledCmps: [],
      enablePrehide: true,
      enableCosmeticRules: true,
      enableGeneratedRules: true,
      enableHeuristicDetection: false,
      enablePopupMutationObserver: false,
      detectRetries: 20,
      isMainWorld: false,
      prehideTimeout: 2e3,
      visualTest: false,
      logs: {
        lifecycle: false,
        rulesteps: false,
        detectionsteps: false,
        evals: false,
        errors: true,
        messages: false,
        waits: false
      },
      performanceLoggingEnabled: false,
      heuristicPopupSearchTimeout: 100,
      enableHeuristicHitTest: false,
      heuristicMode: "off"
      // heuristic disabled by default
    };
    const updatedConfig = copyObject(defaultConfig);
    for (const key of Object.keys(defaultConfig)) {
      if (typeof providedConfig[key] !== "undefined") {
        updatedConfig[key] = providedConfig[key];
      }
    }
    return updatedConfig;
  }
  function scheduleWhenIdle(callback, timeout = 500) {
    if (globalThis.requestIdleCallback) {
      requestIdleCallback(callback, { timeout });
    } else {
      setTimeout(callback, 0);
    }
  }
  function highlightNode(node) {
    const highlightedNode = node;
    if (!node.style) return;
    if (highlightedNode.__oldStyles !== void 0) {
      return;
    }
    if (node.hasAttribute("style")) {
      highlightedNode.__oldStyles = node.style.cssText;
    }
    node.style.animation = "pulsate .5s infinite";
    node.style.outline = "solid red";
    let styleTag = document.querySelector("style#autoconsent-debug-styles");
    if (!styleTag) {
      styleTag = document.createElement("style");
      styleTag.id = "autoconsent-debug-styles";
    }
    styleTag.textContent = `
      @keyframes pulsate {
        0% {
          outline-width: 8px;
          outline-offset: -4px;
        }
        50% {
          outline-width: 4px;
          outline-offset: -2px;
        }
        100% {
          outline-width: 8px;
          outline-offset: -4px;
        }
      }
    `;
    document.head.appendChild(styleTag);
  }
  function unhighlightNode(node) {
    const highlightedNode = node;
    if (!node.style || !node.hasAttribute("style")) return;
    if (highlightedNode.__oldStyles !== void 0) {
      node.style.cssText = highlightedNode.__oldStyles;
      delete highlightedNode.__oldStyles;
    } else {
      node.removeAttribute("style");
    }
  }
  function isTopFrame() {
    return window.top === window && (!globalThis.location.ancestorOrigins || globalThis.location.ancestorOrigins.length === 0);
  }

  // lib/eval-snippets.ts
  var snippets = {
    // code-based rules
    EVAL_0: () => console.log(1),
    EVAL_DIDOMI_OPT_OUT: () => {
      if (window.Didomi) {
        window.Didomi.setUserDisagreeToAll();
        return true;
      }
      return false;
    },
    EVAL_DIDOMI_TEST: () => {
      const purposes = window.Didomi?.getCurrentUserStatus?.()?.purposes;
      if (purposes) {
        return Object.values(purposes).some((p) => !p.enabled);
      }
      const disabled = window.Didomi?.getUserConsentStatusForAll?.()?.purposes?.disabled;
      return Array.isArray(disabled) && disabled.length > 0;
    },
    EVAL_CONSENTMANAGER_1: () => {
      const cmpData = window.__cmp?.("getCMPData");
      return !!cmpData && typeof cmpData === "object";
    },
    EVAL_CONSENTMANAGER_2: () => window.__cmp?.("consentStatus")?.userChoiceExists === false,
    EVAL_CONSENTMANAGER_3: () => __cmp("setConsent", 0),
    EVAL_CONSENTMANAGER_4: () => __cmp("setConsent", 1),
    EVAL_CONSENTMANAGER_5: () => __cmp("consentStatus").userChoiceExists,
    EVAL_CONSENTMANAGER_NCMP_REJECT: () => {
      if (typeof window.__npcmp !== "function") return false;
      window.__npcmp("reject");
      return true;
    },
    EVAL_CONSENTMANAGER_NCMP_ACCEPT: () => {
      if (typeof window.__npcmp !== "function") return false;
      window.__npcmp("save");
      return true;
    },
    EVAL_COOKIEBOT_1: () => !!window.Cookiebot,
    EVAL_COOKIEBOT_2: () => !window.Cookiebot.hasResponse && window.Cookiebot.dialog?.visible === true,
    EVAL_COOKIEBOT_3: () => window.Cookiebot.withdraw() || true,
    EVAL_COOKIEBOT_4: () => window.Cookiebot.hide() || true,
    EVAL_COOKIEBOT_5: () => window.Cookiebot.declined === true,
    EVAL_KLARO_1: () => {
      const config = globalThis.klaroConfig || globalThis.klaro?.getManager && globalThis.klaro.getManager().config;
      if (!config) {
        return true;
      }
      const optionalServices = (config.services || config.apps).filter((s) => !s.required).map((s) => s.name);
      if (klaro && klaro.getManager) {
        const manager = klaro.getManager();
        return optionalServices.every((name) => !manager.consents[name]);
      } else if (klaroConfig && klaroConfig.storageMethod === "cookie") {
        const cookieName = klaroConfig.cookieName || klaroConfig.storageName;
        const consents = JSON.parse(
          decodeURIComponent(
            document.cookie.split(";").find((c) => c.trim().startsWith(cookieName)).split("=")[1]
          )
        );
        return Object.keys(consents).filter((k) => optionalServices.includes(k)).every((k) => consents[k] === false);
      }
    },
    EVAL_KLARO_OPEN_POPUP: () => {
      klaro.show(void 0, true);
    },
    EVAL_KLARO_TRY_API_OPT_OUT: () => {
      if (window.klaro && typeof klaro.show === "function" && typeof klaro.getManager === "function") {
        try {
          klaro.getManager().changeAll(false);
          klaro.getManager().saveAndApplyConsents();
          return true;
        } catch (e) {
          console.warn(e);
          return false;
        }
      }
      return false;
    },
    EVAL_ONETRUST_1: () => window.OnetrustActiveGroups.split(",").filter((s) => s.length > 0).length <= 1,
    EVAL_TRUSTARC_TOP: () => window && window.truste && window.truste.eu.bindMap.prefCookie === "0",
    EVAL_TRUSTARC_FRAME_TEST: () => window && window.QueryString && window.QueryString.preferences === "0",
    EVAL_TRUSTARC_FRAME_GTM: () => window && window.QueryString && window.QueryString.gtm === "1",
    // declarative rules
    EVAL_ADOPT_TEST: () => !!localStorage.getItem("adoptConsentMode"),
    EVAL_ADULTFRIENDFINDER_TEST: () => !!localStorage.getItem("cookieConsent"),
    EVAL_AYLO_COOKIE_MANAGER_READY: () => !!(window.wl_cookie_consent_manager || window._Cookie_Consent_Manager_brand),
    EVAL_BAHN_TEST: () => utag.gdpr.getSelectedCategories().length === 1,
    // the close handler listens for mouseup, which element.click() does not dispatch
    EVAL_BING_NOTICE_CLOSE: () => {
      const close = document.querySelector("#bnp_ttc_close");
      close.dispatchEvent(new MouseEvent("mouseup", { bubbles: true, cancelable: true, view: window }));
      close.click();
      return true;
    },
    EVAL_BIGCOMMERCE_CONSENT_MANAGER_DETECT: () => !!(window.consentManager && window.consentManager.version),
    EVAL_BORLABS_0: () => !JSON.parse(
      decodeURIComponent(
        document.cookie.split(";").find((c) => c.indexOf("borlabs-cookie") !== -1).split("=", 2)[1]
      )
    ).consents.statistics,
    EVAL_CC_BANNER2_0: () => !!document.cookie.match(/sncc=[^;]+D%3Dtrue/),
    EVAL_COINBASE_0: () => JSON.parse(decodeURIComponent(document.cookie.match(/cm_(eu|default)_preferences=([0-9a-zA-Z\\{\\}\\[\\]%:]*);?/)[2])).consent.length <= 1,
    // the close icon is an SVG, which has no click() method
    EVAL_COOKIE_POPUP_CLOSE: () => document.querySelector("#cookie-popup #close-cookies").dispatchEvent(new MouseEvent("click", { bubbles: true })) || true,
    EVAL_COOKIE_LAW_INFO_0: () => {
      if (CLI.disableAllCookies) CLI.disableAllCookies();
      if (CLI.reject_close) CLI.reject_close();
      document.body.classList.remove("cli-barmodal-open");
      return true;
    },
    EVAL_COOKIE_LAW_INFO_DETECT: () => !!window.CLI,
    EVAL_COOKIE_MANAGER_POPUP_0: () => JSON.parse(
      document.cookie.split(";").find((c) => c.trim().startsWith("CookieLevel")).split("=")[1]
    ).social === false,
    EVAL_COOKIEALERT_0: () => document.querySelector("body").removeAttribute("style") || true,
    EVAL_COOKIEALERT_1: () => document.querySelector("body").removeAttribute("style") || true,
    EVAL_COOKIEALERT_2: () => window.CookieConsent.declined === true,
    EVAL_COOKIEFIRST_0: () => ((o) => o.performance === false && o.functional === false && o.advertising === false)(
      JSON.parse(
        decodeURIComponent(
          document.cookie.split(";").find((c) => c.indexOf("cookiefirst") !== -1).trim()
        ).split("=")[1]
      )
    ),
    EVAL_COOKIEFIRST_1: () => document.querySelectorAll("button[data-cookiefirst-accent-color=true][role=checkbox]:not([disabled])").forEach((i) => i.getAttribute("aria-checked") === "true" && i.click()) || true,
    EVAL_COOKIEINFORMATION_0: () => CookieInformation.declineAllCategories() || true,
    EVAL_COOKIEINFORMATION_1: () => CookieInformation.submitAllCategories() || true,
    // banner button order and meaning are configurable per site, so set the consent levels through the public API
    EVAL_COOKIEREPORTS_OPT_OUT: () => {
      const cr = window._cookiereports;
      if (typeof cr?.saveConsent !== "function") return false;
      cr.saveConsent({ 1: true, 2: false, 3: false, 4: false, 5: false });
      cr.processConsent("banner", { trigger: "ok" });
      cr.hideBanner();
      return true;
    },
    EVAL_COOKIEREPORTS_OPT_IN: () => {
      const cr = window._cookiereports;
      if (typeof cr?.saveConsent !== "function") return false;
      cr.saveConsent({ 1: true, 2: true, 3: true, 4: true, 5: true });
      cr.processConsent("banner", { trigger: "ok" });
      cr.hideBanner();
      return true;
    },
    EVAL_COOKIEREPORTS_TEST: () => {
      const levels = window._cookiereports?.loadConsent?.();
      return !!levels && Object.keys(levels).length > 0 && Object.values(levels).every((v) => !v);
    },
    // turn the single-button form into a decline submission, same fields as the CMP's own decline form
    EVAL_EE_CONSENT_DECLINE_0: () => document.querySelectorAll('#cookieConsentFormAccept input[name^="ee:cookies_"]').forEach((i) => i.value = "n") || true,
    EVAL_ETSY_0: () => document.querySelectorAll(".gdpr-overlay-body input").forEach((toggle) => {
      toggle.checked = false;
    }) || true,
    EVAL_ETSY_1: () => document.querySelector(".gdpr-overlay-view button[data-wt-overlay-close]").click() || true,
    EVAL_EZOIC_0: () => ezCMP.handleAcceptAllClick(),
    EVAL_FIDES_DETECT_POPUP: () => window.Fides?.initialized,
    EVAL_GDPR_LEGAL_COOKIE_DETECT_CMP: () => !!window.GDPR_LC,
    EVAL_GDPR_LEGAL_COOKIE_TEST: () => !!window.GDPR_LC?.userConsentSetting,
    EVAL_IUBENDA_0: () => document.querySelectorAll(".purposes-item input[type=checkbox]:not([disabled])").forEach((x) => {
      if (x.checked) x.click();
    }) || true,
    EVAL_IUBENDA_1: () => !!document.cookie.match(/_iub_cs-\d+=/),
    EVAL_KROWN_COOKIE_BANNER_TEST: () => localStorage.getItem("krown-cookie-banner") === "true",
    EVAL_MICROSOFT_0: () => Array.from(document.querySelectorAll("div > button")).filter((el) => el.innerText.match("Reject|Ablehnen"))[0].click() || true,
    EVAL_MICROSOFT_1: () => Array.from(document.querySelectorAll("div > button")).filter((el) => el.innerText.match("Accept|Annehmen"))[0].click() || true,
    EVAL_MICROSOFT_2: () => !!document.cookie.match("MSCC|GHCC"),
    EVAL_MOOVE_0: () => document.querySelectorAll("#moove_gdpr_cookie_modal input").forEach((i) => {
      if (!i.disabled) i.checked = i.name === "moove_gdpr_strict_cookies" || i.id === "moove_gdpr_strict_cookies";
    }) || true,
    EVAL_NHNIEUWS_TEST: () => !!localStorage.getItem("psh:cookies-seen"),
    EVAL_OSANO_DETECT: () => !!window.Osano?.cm?.dialogOpen,
    EVAL_PANDECTES_TEST: () => document.cookie.includes("_pandectes_gdpr=") && JSON.parse(
      atob(
        document.cookie.split(";").find((s) => s.trim().startsWith("_pandectes_gdpr")).split("=")[1]
      )
    ).status === "deny",
    EVAL_POVR_GOBACK: () => window.history.back() || true,
    EVAL_PUBTECH_0: () => document.cookie.includes("euconsent-v2") && (document.cookie.match(/.YAAAAAAAAAAA/) || document.cookie.match(/.aAAAAAAAAAAA/) || document.cookie.match(/.YAAACFgAAAAA/)),
    EVAL_SHOPIFY_TEST: () => document.cookie.includes("gdpr_cookie_consent=0") || document.cookie.includes("_tracking_consent=") && JSON.parse(
      decodeURIComponent(
        document.cookie.split(";").find((s) => s.trim().startsWith("_tracking_consent")).split("=")[1]
      )
    ).purposes.a === false,
    EVAL_SKYSCANNER_TEST: () => document.cookie.match(/gdpr=[^;]*adverts:::false/) && !document.cookie.match(/gdpr=[^;]*init:::true/),
    EVAL_SIRDATA_UNBLOCK_SCROLL: () => {
      document.documentElement.classList.forEach((cls) => {
        if (cls.startsWith("sd-cmp-")) document.documentElement.classList.remove(cls);
      });
      return true;
    },
    EVAL_SOURCEPOINT_RESTORE_SCROLL: () => {
      const root = document.documentElement;
      const saved = root.dataset.previousScrollY;
      if (saved && saved !== "null") {
        root.dataset.previousScrollY = "null";
        const y = -parseInt(saved, 10);
        if (y > 0 && window.scrollY === 0) {
          const behavior = root.style.scrollBehavior;
          root.style.scrollBehavior = "auto";
          window.scrollTo(0, y);
          root.style.scrollBehavior = behavior;
        }
      }
      window.addEventListener("pagehide", () => {
        if (root.dataset.previousScrollY) root.dataset.previousScrollY = "null";
      });
      return true;
    },
    EVAL_STEAMPOWERED_0: () => JSON.parse(
      decodeURIComponent(
        document.cookie.split(";").find((s) => s.trim().startsWith("cookieSettings")).split("=")[1]
      )
    ).preference_state === 2,
    EVAL_TAKEALOT_0: () => document.body.classList.remove("freeze") || (document.body.style = "") || true,
    EVAL_TARTEAUCITRON_0: () => tarteaucitron.userInterface.respondAll(false) || true,
    EVAL_TARTEAUCITRON_1: () => tarteaucitron.userInterface.respondAll(true) || true,
    EVAL_TARTEAUCITRON_2: () => document.cookie.match(/tarteaucitron=[^;]*/)?.[0].includes("false"),
    EVAL_TEALIUM_0: () => typeof window.utag !== "undefined" && typeof utag.gdpr === "object",
    EVAL_TEALIUM_1: () => utag.gdpr.setConsentValue(false) || true,
    EVAL_TEALIUM_DONOTSELL: () => utag.gdpr.dns?.setDnsState(false) || true,
    EVAL_TEALIUM_2: () => utag.gdpr.setConsentValue(true) || true,
    EVAL_TEALIUM_3: () => utag.gdpr.getConsentState() !== 1,
    EVAL_TEALIUM_DONOTSELL_CHECK: () => utag.gdpr.dns?.getDnsState() !== 1,
    // 0 means no stored decision; otherwise the user reopened the prompt themselves
    EVAL_TEALIUM_NO_DECISION: () => utag.gdpr.getConsentState() === 0,
    EVAL_TESTCMP_STEP: () => !!document.querySelector("#reject-all"),
    EVAL_TESTCMP_0: () => window.results.results[0] === "button_clicked",
    EVAL_TESTCMP_COSMETIC_0: () => window.results.results[0] === "banner_hidden",
    EVAL_THEFREEDICTIONARY_0: () => cmpUi.showPurposes() || cmpUi.rejectAll() || true,
    EVAL_THEFREEDICTIONARY_1: () => cmpUi.allowAll() || true,
    EVAL_USERCENTRICS_API_0: () => typeof UC_UI === "object",
    EVAL_USERCENTRICS_API_1: () => !!UC_UI.closeCMP(),
    EVAL_USERCENTRICS_API_2: () => !!UC_UI.denyAllConsents(),
    EVAL_USERCENTRICS_API_3: () => !!UC_UI.acceptAllConsents(),
    EVAL_USERCENTRICS_API_4: () => !!UC_UI.closeCMP(),
    EVAL_USERCENTRICS_API_5: () => UC_UI.areAllConsentsAccepted() === true,
    EVAL_USERCENTRICS_API_6: () => UC_UI.areAllConsentsAccepted() === false,
    EVAL_USERCENTRICS_BUTTON_0: () => JSON.parse(localStorage.getItem("usercentrics")).consents.every((c) => c.isEssential || !c.consentStatus),
    EVAL_WAITROSE_0: () => Array.from(document.querySelectorAll("label[id$=cookies-deny-label]")).forEach((e) => e.click()) || true,
    EVAL_WBD_LTP_CLOSE: () => document.querySelector("dialog#ltp-dialog").close() || true,
    EVAL_ZOHO_TEST: () => localStorage.getItem("zglobal_Acookie_optOut") === "3"
  };
  function getFunctionBody(snippetFunc) {
    const snippetStr = snippetFunc.toString();
    return `(${snippetStr})()`;
  }

  // lib/heuristic-patterns.ts
  var patternOf = (p) => p instanceof RegExp ? p : p.pattern;
  var BY_CONTINUING_PRIVACY = /(?:by continuing.{0,100}privacy)|(?:privacy.{0,100}by continuing)/gi;
  var USE_OF_COOKIES = /use of cookies/gi;
  var UTILISATION_DES_COOKIES = /utilisation.{0,100}des.{0,100}cookies/gi;
  var ACCEPTEZ_UTILISATION = /acceptez.{0,100}l.{0,100}utilisation/gi;
  var VERWENDUNG_VON_COOKIES = /verwendung.{0,100}von.{0,100}cookies/gi;
  var DE_COOKIE_VERB = /\b(?:nutzen|nutzt|verwenden|verwendet|setzen|setzt|benutzen|benutzt|speichert|platzieren)\b(?: \S+){0,4} cookies\b/gi;
  var GEBRUIK_VAN_COOKIES = /gebruik.{0,100}van.{0,100}cookies/gi;
  var USO_DEI_COOKIE = /\b(?:uso|utilizzo) dei cookies?\b/gi;
  var ANVANDNING_AV_COOKIES = /\b(?:användning|användningen|användandet) av (?:\S+ )?(?:cookies|kakor)\b/gi;
  var DE_THIS_SITE_COOKIES = /\b(?:unsere|diese)[rs]? (?:web)?(?:seite|site)\b.{0,100}cookies/gi;
  var DE_SITE_USES_COOKIES = /\b(?:seite|site)\b.{0,100}\b(?:nutzt|benutzt)\b.{0,100}cookies/gi;
  var DETECT_PATTERNS = [
    // no bare "accept all" / "reject all" labels in any language: page content with bulk actions has them too
    /accept cookies/gi,
    /only necessary cookies/gi,
    // "only necessary" is probably too broad
    /(?:by continuing.{0,100}cookie)|(?:cookie.{0,100}by continuing)/gi,
    /\bby using (?:this|our) (?:web)?site\b.{0,100}cookie/gi,
    // "By clicking 'Accept all', you agree to the storing of cookies …"
    /\bby clicking\b.{0,80}?\byou (?:allow|accept|agree|consent)\w*\b.{0,100}?\b(?:cookies?|technologies)\b/gi,
    // button labels count only with the cookie noun: bare "accept all" / "reject all" also label bulk actions in page content
    /accept all cookies/gi,
    /\breject non-(?:essential|necessary)\b/gi,
    /\bwhich (?:types? of )?cookies (?:that )?you (?:accept|allow|want)\b/gi,
    /\bstores? cookies on your (?:device|computer|browser)\b/gi,
    /\byour consent to (?:collect|use|store|process|set)\b/gi,
    /\byou agree to (?:our |the )?cookie use\b/gi,
    /\bblocking some types of cookies\b/gi,
    // consent management platform footers
    /\bpowered by usercentrics\b|\b(?:realisiert mit|realized with) klaro\b/gi,
    BY_CONTINUING_PRIVACY,
    // up to two qualifiers are common: "optional", "essential", "analytics", "google analytics", "third party", ...
    // singular too, but not "cookie-free"
    /we (?:also )?(?:use|serve)(?: \w+){0,2} cookies?\b(?!-)/gi,
    /\bwe use (?:\S+ )?cookie files\b/gi,
    // consent asked as a question: "can I use cookies to …?", "allow analytics cookies?"
    /\b(?:can|may) (?:i|we) use (?:\S+ ){0,2}cookies\b/gi,
    /\ballow (?:\S+ )?cookies\?/gi,
    /we are using cookies/gi,
    USE_OF_COOKIES,
    /website uses cookies to enhance your browsing experience/gi,
    /(?:this|our) (?:web)?site.{0,100}cookies/gi,
    /cookies (?:and|or) .{0,100} technologies/gi,
    /such as cookies/gi,
    /read more about.{0,100}cookies/gi,
    /consent to.{0,100}cookies/gi,
    /we and our (?:\w+ )?partners.{0,100}cookies/gi,
    /we.{0,100}store.{0,100}information.{0,100}such as.{0,100}cookies/gi,
    /store and\/or access information.{0,100}on a device/gi,
    /personalised ads and content, ad and content measurement/gi,
    // TCF wording: "store and/or access information", "and our partners ask for your consent", "we and our N partners"
    /\b(?:store|stores|storing) and\/or access(?:ing)? information\b/gi,
    /\band our partners ask for your consent\b/gi,
    /\bwe and (?:our|up to) (?:\d+ )?(?:partners|vendors)\b/gi,
    // a partner count, in each language: "our 186 partners", "unseren 107 Partnern", "nuestros 1064 socios"
    /\b(?:our|unseren|onze|nos|nuestros|i nostri) \d{2,4} (?:partners|partnern|partner|partenaires|socios|vendors|fornitori)\b/gi,
    /\b\d{2,4} iab[- ]partner/gi,
    // "technologies like cookies", in each language
    /\btechnolog(?:ies|ien|ieën|ías|ie) (?:like|such as|wie|zoals|como(?: las)?|telles que(?: les)?|come(?: i)?) cookies?\b/gi,
    /\buses (?:\S+ ){0,2}cookies\b/gi,
    /\b(?:like|want) to (?:use|set) (?:\S+ ){0,2}cookies\b/gi,
    /\b(?:optional|additional|non-essential) cookies\b/gi,
    /\bstoring of cookies\b|\bplaced cookies\b/gi,
    /\b(?:store|stores) and access (?:data|information) on your device\b|\baccess and store (?:data|information) on your device\b/gi,
    /\bwe use (?:\S+ ){1,4}cookies\b/gi,
    // US privacy notices: opt-out sentences, not the footer link labels
    /\byou (?:can|may) opt[- ]out of (?:the |this )?(?:sale|sharing|use|selling)\b/gi,
    /\b(?:if you wish to|to) opt[- ]out of the sale\b/gi,
    /\bthis website or its third[- ]party tools (?:process|use)\b/gi,
    /["“]?(?:share|sell)["”]? or ["“]?(?:sell|share)["”]? (?:your )?(?:personal )?information\b/gi,
    /\bresidents have (?:certain )?rights with regard to the sale\b/gi,
    /\bwe use technologies (?:on this website )?(?:that may )?(?:to )?collect\b/gi,
    /\bwe use third[- ]party tools to collect information\b/gi,
    // pay-or-consent walls
    /\bcontinue reading with advertising\b|\b(?:with|mit) (?:advertising|werbung) (?:and|und) tracking\b/gi,
    /\bconsent and continue\b/gi,
    // preference centres are often in the page but hidden, so popup text only
    { pattern: /\bstore or retrieve information on your browser\b|\bstockées dans votre navigateur ou récupérées\b/gi, input: "popup" },
    /\bwe have cookies\b/gi,
    /\b(?:revoke|withdraw) your consent at any time\b|\beinwilligung jederzeit widerrufen\b/gi,
    /\bwe use (?:[\w-]+ )?analytics (?:to|for)\b|\bwe use google analytics\b.{0,100}\bcookie/gi,
    /\b(?:con|senza) il (?:tuo|vostro) consenso\b/gi,
    /\baccept or (?:reject|decline) (?:all |optional |non-essential )?cookies\b/gi,
    // it might be tempting to add the patterns below, but they cause too many false positives. Don't do it :)
    // /cookies? settings/i,
    // /cookies? preferences/i,
    // FR
    /utilisons.{0,100}des.{0,100}cookies/gi,
    /des.{0,100}cookies.{0,100}pour/gi,
    /retirer.{0,100}votre.{0,100}consentement/gi,
    /et.{0,100}nos.{0,100}partenaires/gi,
    /publicités.{0,100}et.{0,100}du.{0,100}contenu/gi,
    /utilise.{0,100}des.{0,100}cookies/gi,
    /utilisent.{0,100}des.{0,100}cookies/gi,
    /stocker.{0,100}et.{0,100}ou.{0,100}accéder/gi,
    /consentement.{0,100}à.{0,100}tout.{0,100}moment/gi,
    /votre.{0,100}consentement/gi,
    UTILISATION_DES_COOKIES,
    /cookies.{0,100}ou.{0,100}technologies/gi,
    ACCEPTEZ_UTILISATION,
    /continuer sans accepter/gi,
    /(?:refuser|rejeter) tous les cookies/gi,
    // not "je refuse d'…" in a headline
    /je refuse(?! d['’e])/gi,
    /refuser et continuer/gi,
    /\ben cliquant sur\b.{0,80}?\bvous (?:acceptez|consentez)\b.{0,60}?\b(?:cookies|traceurs)\b/gi,
    /\bvous consentez\b.{0,60}\bcookies\b/gi,
    /\baccepter ou refuser (?:\S+ ){0,3}cookies\b/gi,
    /\bles cookies nous (?:sont|permettent|aident)\b/gi,
    /\btraceurs (?:optionnels|publicitaires|non essentiels)\b/gi,
    /seulement nécessaires/gi,
    /je désactive les finalités non essentielles/gi,
    /cookies essentiels uniquement/gi,
    /nécessaires uniquement/gi,
    /\b(?:stocker|stockons) et\/ou (?:accéder|accédons)\b/gi,
    /\bnos partenaires demandent votre consentement\b/gi,
    /\bconsentez à l.utilisation (?:de |des )?cookies\b|\bdépôt (?:de|des) cookies\b/gi,
    /\bles cookies nous permettent\b|\bnos partenaires et nous(?:-mêmes)?\b/gi,
    /\baccepter (?:&|et) fermer\b/gi,
    // up to two qualifiers, as in English: "nous utilisons nos propres cookies", "nous utilisons des cookies tiers"
    /\b(?:on utilise|nous utilisons)(?: \S+){0,2} cookies\b/gi,
    /\ben (?:naviguant|poursuivant)\b.{0,100}cookies/gi,
    /(?:en continuant.{0,100}cookie)|(?:cookie.{0,100}en continuant)/gi,
    /\ben (?:naviguant|poursuivant|continuant)\b.{0,100}confidentialité/gi,
    // DE
    /wir.{0,100}verwenden.{0,100}cookies/gi,
    /wir.{0,100}und.{0,100}unsere.{0,100}partner/gi,
    /zugriff auf informationen auf/gi,
    /inhalte.{0,100}messung.{0,100}von.{0,100}werbeleistung.{0,100}und/gi,
    /cookies.{0,100}und.{0,100}andere/gi,
    VERWENDUNG_VON_COOKIES,
    /wir.{0,100}nutzen.{0,100}cookies/gi,
    /verwendet.{0,100}cookies/gi,
    /sie.{0,100}können.{0,100}ihre.{0,100}auswahl/gi,
    /und.{0,100}ähnliche.{0,100}technologien/gi,
    /cookies.{0,100}wir.{0,100}verwenden/gi,
    /(?:nur|nicht).{0,100}(?:zusätzliche|essenzielle|funktionale|notwendige|erforderliche).{0,100}(?:cookies|akzeptieren|erlauben|ablehnen)/gi,
    /weiter.{0,100}(?:ohne|mit).{0,100}(?:einwilligung|zustimmung|cookies)/gi,
    /(?:cookies|einwilligung).{0,100}ablehnen/gi,
    /nur funktionale cookies akzeptieren/gi,
    /optionale ablehnen/gi,
    /zustimmung verweigern/gi,
    /\bspeichern und\/oder (?:zugreifen|abrufen)\b/gi,
    /\bunsere partner bitten um ihre zustimmung\b/gi,
    DE_COOKIE_VERB,
    /\b(?:werden|wird) (?:\S+ ){0,3}cookies (?:verwendet|eingesetzt|gesetzt|genutzt)\b/gi,
    /\bcookies? zum einsatz\b|\beinsatz von cookies\b/gi,
    /\beinverstanden\b.{0,60}\bcookies\b|\bcookies\b.{0,80}\beinverstanden\b/gi,
    /\b(?:alle|alles) (?:cookies|optionalen \S+) (?:akzeptieren|zulassen|erlauben|zustimmen)\b/gi,
    /\bzustimmen und weiter\b|\bweiter,? ohne zu (?:akzeptieren|zustimmen)\b/gi,
    /\b(?:einwilligung|zustimmung) (?:zu|zur|in die) (?:\S+ )?cookies\b|\bcookie-(?:zustimmung|einwilligung)\b/gi,
    /\btracking-technologien\b/gi,
    DE_SITE_USES_COOKIES,
    // verb before the subject: "Für die Analyse nutzen wir Cookies"
    /\b(?:verwenden|nutzen|benutzen|setzen) wir\b.{0,100}cookies/gi,
    /\bwir\b.{0,100}\bsetzen\b.{0,100}cookies/gi,
    /\bbenutz(?:t|en)\b.{0,100}cookies/gi,
    /\bdurch die (?:weitere )?nutzung (?:unserer|dieser) (?:webseite|website|seite)\b.{0,100}datenschutz/gi,
    /\bakzeptier\w* (?:du |sie )?(?:die )?verwendung (?:\S+ )?cookies\b/gi,
    /\bmit werbetracking weiterlesen\b|\bweiter mit werbung\b|\bcon tracking e annunci\b/gi,
    // in page text, a footer line such as "Diese Website verwendet Cookies. Mehr erfahren" needs a consent control nearby (see heuristic-negatives.ts)
    DE_THIS_SITE_COOKIES,
    /(?:durch die weitere nutzung.{0,100}cookie)|(?:cookie.{0,100}durch die weitere nutzung)/gi,
    /\b(?:analyse|statistik)-?cookies\b/gi,
    /\b(?:marketing|tracking)-?cookies\b/gi,
    /\bcookies oder (?:andere|ähnliche)/gi,
    // "Wenn Sie auf „Alle akzeptieren“ klicken, stimmen Sie …", "Durch Klicken auf „Alle akzeptieren“ gestatten Sie …"
    /\b(?:indem sie auf|wenn sie auf|durch (?:das )?klicken auf)\b.{0,80}?\b(?:stimmen sie|gestatten sie|willigen sie|erklären sie sich)\b/gi,
    /\bbenötigen wir (?:ihre|deine) (?:einwilligung|zustimmung)\b/gi,
    {
      pattern: /\b(?:einwilligung|zustimmung)\b.{0,60}\b(?:jederzeit|widerrufen)\b|\bmit (?:ihrer|deiner) (?:einwilligung|zustimmung)\b/gi,
      input: "popup"
    },
    /\bwelche cookies sie (?:zulassen|akzeptieren)\b/gi,
    /\bcookies?\b.{0,60}\bauf (?:ihrem|deinem) (?:gerät|endgerät)\b/gi,
    /\bin form von cookies\b|\bblockierung bestimmter arten von cookies\b/gi,
    /\b(?:notwendige|essenzielle|funktionale) cookies\b/gi,
    // innerText can put every word on its own line
    /\bwir\s+(?:verwenden|nutzen|setzen)\s+cookies\b/gi,
    /\bzwecken und anbietern\b/gi,
    /\bweiter ohne akzeptanz\b/gi,
    // NL
    GEBRUIK_VAN_COOKIES,
    /(?:we|wij).{0,100}gebruiken.{0,100}cookies.{0,100}om/gi,
    /cookies.{0,100}en.{0,100}vergelijkbare/gi,
    /cookies.{0,100}(?:afwijzen|weigeren|verwerpen)/gi,
    /alleen.{0,100}noodzakelijke?\b/gi,
    /cookies weigeren/gi,
    /weiger.{0,100}cookies/gi,
    /doorgaan zonder (?:te accepteren|akkoord te gaan)/gi,
    /alleen.{0,100}(?:optionele|functionele|functioneel|noodzakelijke|essentiële|standaard).{0,100}cookies/gi,
    /\b(?:op te slaan en\/of te (?:raadplegen|openen|lezen)|opslaan en\/of (?:openen|raadplegen|lezen)|slaan informatie\b.{0,60}\bop en\/of)\b/gi,
    /\bonze partners vragen om (?:jouw|uw|je) toestemming\b/gi,
    /\bwij en onze (?:\d+ )?partners\b/gi,
    /\b(?:gebruiken|gebruikt|plaatsen|plaatst)\b(?: \S+){0,3} cookies\b/gi,
    /\baccepteer alle cookies\b/gi,
    // Dutch words only: "tracking cookies" and "marketing cookies" are also English prose and footer links
    /\b(?:analytische|functionele|essentiële|noodzakelijke) cookies\b/gi,
    /(?:te blijven gebruiken.{0,100}cookie)|(?:cookie.{0,100}te blijven gebruiken)/gi,
    /cookies.{0,100}beste? (?:mogelijke )?(?:ervaring|gebruikerservaring)/gi,
    /\b(?:we|wij) (?:gebruiken|plaatsen)\b.{0,100}cookies/gi,
    /\bgebruiken (?:we|wij) cookies\b/gi,
    /\bplaatsen van\b.{0,50}cookies/gi,
    /\bte blijven gebruiken\b.{0,100}privacy/gi,
    /\bdoor op\b.{0,60}?\bte klikken\b.{0,40}?\bga(?:at)? (?:u|je) akkoord\b/gi,
    /\balle cookies accepteren\b/gi,
    /\btoestemming voor (?:\S+ ){0,3}cookies\b/gi,
    // Spanish (ES)
    /(si|al) contin[úu]a[sr]?( navegando)?.{0,100} cookie/gi,
    /(usamos|utilizar?|utilizamos)( (tanto|las))?.{0,20}cookie/gi,
    /\b(hacemos|hace) uso de cookies\b/gi,
    /\busa cookies de google\b/gi,
    /acepta.{0,80} uso de cookies/gi,
    /al utilizar nuestro sitio web.{0,80}cookie/gi,
    /almacenar la información en un dispositivo y\/?o acceder a ella/gi,
    /cookie.{0,30} utiliza/gi,
    /cookies propias y de/gi,
    /cookies.{0,80}son necesarias/gi,
    /est[ea] (sitio|página|web)( web)?( también)? (usa|utiliza|requiere del uso de|se sirven|emplea) cookies?/gi,
    /navegando.{0,100}cookie/gi,
    /nosotros y nuestros( \d+)? (socios|proveedores).{0,180} cookies/gi,
    /recopilamos y almacenamos datos de usted y de su dispositivo/gi,
    /utilizamos tecnolog[ií]as como las cookies/gi,
    /\b(?:almacenar|almacenamos) y\/o (?:acceder|accedemos)\b/gi,
    /\bnuestros socios solicitan su consentimiento\b/gi,
    /\b(?:tanto nosotros como|nosotros y) nuestros (?:\d+ )?socios\b/gi,
    /\bcontinuar sin aceptar\b|\bsolo las necesarias\b/gi,
    /\bacept\w* (?:\S+ ){0,3}uso de (?:las )?cookies\b/gi,
    /\bcookies (?:técnicas|analíticas|propias|de terceros|opcionales|necesarias)\b/gi,
    /\blas cookies nos (?:ayudan|permiten)\b/gi,
    /\b(?:instalamos|empleamos)\b(?: \S+){0,4} cookies\b/gi,
    /\bacept(?:a|ar|as)? (?:el uso de |las )?(?:\S+ )?cookies\b|\bnecesitamos usar cookies\b/gi,
    /\bal hacer clic en\b.{0,80}?\bcookies\b/gi,
    /\b(?:aceptar|rechazar) todas las cookies\b/gi,
    /\bestrictamente necesarias\b/gi,
    /\balmacenar o acceder a (?:la )?información\b/gi,
    /\btecnolog[ií]as de seguimiento\b/gi,
    // Polish (PL)
    // examples:
    //  wykorzystuje pliki cookie (uses cookies)
    //  Wykorzystujemy informacje w plikach cookie (We use information in cookies)
    /(używamy|stosujemy|stosuje|wykorzystujemy|wykorzyst(uje|ywane))( są)?.{0,20} plik(i|ów|ach) [„"]?cookie/gi,
    /(używać|używamy).{0,80} (ciasteczek|cookie)/gi,
    /cele przetwarzania twoich danych przez zaufanych partnerów iab/gi,
    /dzięki (plikom cookie|ciasteczkom|cookie)/gi,
    /korzysta.{0,80} plików cookie/gi,
    /korzystamy z technologii, takich jak pliki cookie/gi,
    /korzystamy.{0,50} cookies/gi,
    /niektóre pliki cookies/gi,
    /pliki cookies i pokrewne im technologie umożliwiają poprawne działanie strony i pomagają nam dostosować ofertę do twoich potrzeb/gi,
    /przechowywanie informacji na urządzeniu lub dostęp do nich/gi,
    /przechowywanie plików cookie na swoim urządzeniu/gi,
    /przechowywać i uzyskiwać dostęp do informacji na twoich urządzeniach/gi,
    /przetwarzamy.{0,80} cookie/gi,
    /strona.{0,50} używa (ciasteczek|cookie)/gi,
    /ta strona korzysta z ciasteczek/gi,
    /uzyskujemy dostęp i przechowujemy informacje na urządzeniu/gi,
    /używa plik[ió]w? cookie/gi,
    /używamy plików.{0,20}cookie/gi,
    /wykorzystują .{0,100}cookie/gi,
    /za pomocą plików cookies.{0,100} my lub nasi partnerzy/gi,
    /zgodą my i nasi partnerzy możemy wykorzystywać precyzyjne dane geolokalizacyjne i identyfikację/gi,
    // Catalan (CA)
    /cookies pròpies i de tercers/gi,
    /utilitzem galetes/gi,
    /únicament utilitza galetes pròpies amb finalitat tècnica\b/gi,
    /este lloc web utilitza només cookies tècniques necessàries per al seu funcionament/gi,
    /utilitza cookies tècniques,\s*de personalització i anàlisi/gi,
    /utilitzem cookies i altres tecnologies/gi,
    /\b(?:utilitz\w*|fa servir|fem servir|usem)\b(?: \S+){0,3} (?:cookies|galetes)\b/gi,
    // Basque (EU)
    /cookie propio eta hirugarrenenak helburu teknikoarekin erabiltzen ditu/gi,
    /cookie propioak eta hirugarrenen cookieak erabiltzen ditugu/gi,
    /cookie propioak eta hirugarrenenak helburu teknikoarekin erabiltzen ditu/gi,
    /cookie[-\s]*ak erabiltzen ditu/gi,
    /cookieak erabiltzen ditu/gi,
    /guk eta gure \d+ bazkideek cookieak eta identifikadoreak erabiltzen ditugu/gi,
    /norberaren eta hirugarrenen cookie-?ak baino ez ditu erabiltzen/gi,
    /web orri honek cookieak erabiltzen ditu/gi,
    /webgune honek cookie propioak eta hirugarrenen cookie-fitxategiak erabiltzen ditu/gi,
    // Galician (GL)
    /^\s*empregamos cookies propias\b/gi,
    /este portal emprega cookies propias ou de terceiros con fins analíticos/gi,
    // Russian (RU)
    // e.g. "мы используем файлы cookie", "сайт использует куки", "используются технологии cookie"
    // note: \w does not match Cyrillic, so these use explicit character ranges
    /использу[а-яё]*.{0,40}(?:файл[а-яё]*[\s-]+)?(?:cookie|куки)/gi,
    /(?:cookie|куки).{0,40}использу[а-яё]*/gi,
    /(?:использовани[а-яё]*|обраб(?:отк|атыва)[а-яё]*|хранени[а-яё]*).{0,20}(?:файл(?:ов|ы)[\s-]+)?(?:cookie|куки)/gi,
    /технологи[а-яё]*\s+(?:cookie|куки)/gi,
    // e.g. "сайт собирает cookie", "мы применяем файлы cookie", "сохраняем куки на вашем устройстве"
    /(?:собира[а-яё]*|сбор|применя[а-яё]*|сохраня[а-яё]*|размеща[а-яё]*|устанавлива[а-яё]*).{0,30}(?:файл[а-яё]*[\s-]+)?(?:cookie|куки)/gi,
    // "файлы cookie" / "cookie-файлы" / "куки-файлы" in any phrasing
    /(?:файл[а-яё]*[\s-]+(?:cookie|куки)|(?:cookie|куки)[\s-]+файл[а-яё]*)/gi,
    // cookies described without the word: "файлов, сохраняемых в браузере пользователя"
    /файл[а-яё]*,?\s+(?:сохраня[а-яё]*|хран[а-яё]*)[а-яё\s]{0,30}браузер/gi,
    // Italian (IT)
    // e.g. "usiamo i cookie", "utilizziamo cookie tecnici", "questo sito utilizza i cookie", "noi e i nostri partner utilizziamo i cookie"
    /(?:usiamo|utilizziamo).{0,20}cookie/gi,
    /sito.{0,50}(?:usa|utilizza).{0,20}cookie/gi,
    /noi e i nostri.{0,20}partner.{0,100}cookie/gi,
    /\b(?:utilizziamo|usiamo|impieghiamo|installiamo)\b(?: \S+){0,3} cookies?\b/gi,
    /\b(?:utilizza|utilizzano|usa|usano|impiega|installa|fa uso di)\b(?: \S+){0,3} cookies?\b/gi,
    /\bcookies? (?:tecnici|di profilazione|di terze parti|analitici|statistici|di marketing)\b/gi,
    /\b(?:acconsent|accett)\w*\b.{0,40}\b(?:uso|utilizzo|installazione)\b.{0,15}\bcookies?\b/gi,
    /\b(?:continuando|proseguendo)(?: la| nella)? navigazione\b.{0,80}\b(?:accett|acconsent)\w*/gi,
    /\b(?:continua|prosegui|chiudi) senza accettare\b/gi,
    /\brifiuta e abbonati\b/gi,
    /\bconsenso (?:all.(?:uso|utilizzo) (?:dei |di )?)?cookies?\b/gi,
    /\b(?:insieme ai|con i) nostri (?:\d+ )?partner\b/gi,
    /\bterze parti selezionate\b/gi,
    /\b(?:archiviare|archiviamo|memorizzare|memorizziamo) e\/o (?:accedere|accediamo)\b/gi,
    USO_DEI_COOKIE,
    /\butilizz\w*\b.{0,30}cookie/gi,
    /\b(?:questo|il nostro) sito\b.{0,100}cookie/gi,
    /\bcookie\b.{0,30}\btecnologie (?:simili|analoghe)\b/gi,
    /\bcliccando su\b.{0,80}?\bcookie/gi,
    /\b(?:accetta|rifiuta) tutti i cookie/gi,
    // TCF: "noi e i nostri partner archiviamo …", "… potremmo utilizzare dati di localizzazione", not "partner commerciali"
    /\bnoi e i nostri (?:\d+ )?partner (?:archivia\w*|memorizz\w*|potremmo|trattiamo)\b/gi,
    /\bcookies? (?:essenziali|necessari)\b/gi,
    // Swedish (SV)
    // e.g. "vi använder cookies", "vi använder spårningstekniker såsom cookies", "webbplatsen använder kakor"
    /(?:vi|webbplatsen|webbplats|sajten|sidan)\s+använder.{0,60}(?:cookies|kakor)/gi,
    // any subject and inverted order: "Svenska kraftnät använder kakor", "använder vi också cookies", "vill använda cookies",
    // "vi sätter inga egna cookies"
    /\b(?:använd(?:er|a)|sätter|nyttjar) (?:vi |sig av |oss av )?(?:\S+ ){0,3}(?:cookies|kakor)\b/gi,
    // passive: "på webbplatsen används kakor", "kakor används för att …"
    /\banvänds (?:\S+ ){0,2}(?:cookies|kakor)\b|\b(?:cookies|kakor)\b.{0,60}\banvänds\b/gi,
    ANVANDNING_AV_COOKIES,
    /\b(?:godkänn|accepter|tillåt)\S* (?:\S+ ){0,4}(?:cookies|kakor)\b|\b(?:cookies|kakor) (?:som )?du tillåter\b/gi,
    // a button or "… nödvändiga kakor", not "endast nödvändiga resurser"
    /\b(?:endast|bara|enbart) nödvändiga(?: (?:cookies|kakor))?(?![ \t]*[\wåäö])/gi,
    // TCF wording
    /\blagra och\/eller (?:få åtkomst till|komma åt) information\b/gi,
    /\b(?:dess|våra) partners?\b.{0,100}\b(?:cookies|kakor)\b/gi,
    /\btekniker (?:såsom|som) (?:cookies|kakor)\b/gi,
    // Japanese (JA)
    // e.g. "当サイトはCookieを使用しています", "クッキー（Cookie）を利用しています"
    /(?:cookie|クッキー).{0,20}を(?:使用|利用)/gi,
    // e.g. "閲覧履歴を外部サービスに送信"
    /(?:閲覧履歴|端末情報|利用者情報).{0,30}を外部(?:サービス|事業者|に|へ)/gi,
    // Danish (DA)
    // e.g. "vi bruger cookies", "hjemmesiden bruger cookies til at ..."
    /\b(?:bruger|anvender|benytter) cookies\b/gi,
    // Norwegian (NO)
    // e.g. "vi bruker informasjonskapsler", "kan vi bruke valgfrie informasjonskapsler", "nettstedet bruker cookies"
    /\b(?:vi|nettstedet|nettsiden|siden)\s+bruker?\s.{0,60}?(?:informasjonskapsler|cookies)/gi,
    /\bvi benytter(?: \S+)? (?:cookies|informasjonskapsler)\b/gi,
    /\bbruk av (?:cookies|informasjonskapsler)\b/gi
  ];
  var REJECT_PATTERNS_ENGLISH = [
    // e.g. "reject", "reject all", "reject all cookies", "reject cookie", "deny all", "refuse cookies", "decline", "disallow cookies",
    // "reject non-essential cookies", "reject unnecessary cookies", "reject all but necessary", "reject all and close"
    // note that "reject and subscribe" and "reject and pay" are excluded via BUTTON_NEVER_MATCH_PATTERNS
    /^\s*(no,?\s*)?(i\s+)?(reject|deny|refuse|decline|disable|disallow)\s*(all)?\s*(but|except)?\s*(non[- ]?essential|un(necessary|required)|optional|additional|targeting|analytics|marketing|non[- ]?necessary|extra|tracking|advertising|necessary|essential)?\s*(cookies?)?\s*(and\s+close)?\s*$/is,
    // e.g. "i do not accept", "do not accept cookies", "i do not accept the use of cookies", "do not allow cookies", "dont allow"
    /^\s*(no,?\s*)?(i\s+)?(do\s+not|dont)\s+(accept|allow)\s*(the\s+use\s+of\s+)?(any\s+)?(cookies)?\s*$/is,
    // e.g. "continue without accepting", "continue without agreeing", "continue without agreeing →"
    /^\s*(continue|proceed|continue\s+browsing)\s+without\s+(accepting|agreeing|consent|cookies|tracking)(\s*→)?\s*$/is,
    // essential/necessary/functional-only, e.g. "essential cookies only", "accept only essential cookies",
    // "allow necessary cookies continue", "use essential cookies only", "functional only", "essential only cookies", "i confirm necessary",
    // "proceed with necessary cookies only"
    /^\s*(i\s+)?(want\s+to\s+)?(only\s+)?(use|accept|allow|keep|enable|choose|continue\s+with|proceed\s+with|i\s+confirm)\s*(only\s+)?(strictly\s+)?(necessary|essential|essentials|functional|required|minimal)\s*(cookies)?\s*(continue|only)?\s*$/is,
    /^\s*(i\s+)?(want\s+to\s+)?only\s+(strictly\s+)?(necessary|essential|essentials|functional|required|minimal)\s*(cookies)?\s*(continue|only)?\s*$/is,
    /^\s*(strictly\s+)?(necessary|essential|essentials|functional|required|minimal)\s+((cookies\s+)?only|only\s+cookies)\s*$/is,
    // e.g. "do not sell or share my personal information", "do not share my personal information", "opt out of sale ..." (CCPA)
    // "do not sell page" is a link; "do not sellcookie settings" is two concatenated buttons
    /do\s+not\s+sell(?!\s*(page|cookie settings))|do\s+not\s+share\s+my\s+(personal\s+)?(information|info|data)|opt\s+out\s+of\s+sale/is,
    // e.g. "opt-out of sale/share or targeted advertising", "opt-out of advertising/social media cookies"
    /^opt[ -]?out of /is,
    // e.g. "reject all except strictly necessary", "reject all (except necessary cookies)"
    /except\s+(strictly\s+)?necessary/is,
    // e.g. "disagree", "i disagree", "disagree and close"
    /^(i\s+)?disagree\s*(and\s+close)?$/i,
    "no",
    /^no,? thank(s| you)$/is,
    // e.g. "no cookies", "no cookies for me, thanks", "rather not", "no, rather not"
    /^no,?\s+cookies(\s+for\s+me)?(,?\s*thank(s| you))?$/is,
    /^(no,?\s+)?rather not$/is,
    /^opt[ -]out$/is,
    "dont enable",
    "i do not agree"
  ];
  var REJECT_PATTERNS_DUTCH = [
    // weigeren / afwijzen / verwerpen (reject verbs, any position)
    /weiger|afwijz|verwerp/is,
    // "wijs (alles/ze) af" (reject verb with split particle)
    /wijs\b.*\baf\b/is,
    // "alleen/enkel/uitsluitend ... (noodzakelijk|functioneel|essentieel|...)"
    /(^|\s)(alleen|enkel|uitsluitend)\s+.{0,20}(noodzakelijk|functione|essenti|vereiste|verplichte|strikt|minimale|basis|basic|standaard)/is,
    // essential/necessary-only nouns
    /^\s*(accepteer\s+|aanvaard\s+|gebruik\s+|sta\s+|ik wil\s+)?(alleen\s+|enkel\s+|uitsluitend\s+|strikt\s+)?(de\s+)?(noodzakelijke?|functionele?|functioneel|essentiële|essentieel|vereiste|verplichte|minimale|basiscookies|basis|standaard)\s*(cookies?)?\s*(accepteren|toestaan|aanvaarden)?\s*$/is,
    // continue without accepting / consent
    /(doorgaan|ga door|ga verder|verder)\s+.{0,15}(zonder|aanvaard)/is,
    // "nee" refusals (but not "nee, sluiten" → acknowledge)
    /^nee(,?\s+(bedankt|dank je|dankje|liever niet|liever geen cookies|geen persoonlijke cookies|geen cookies.*|weigeren?))?$/is,
    "niet accepteren",
    "niet akkoord",
    "ik ga niet akkoord",
    "liever niet",
    "geen cookies toestaan",
    "liever geen cookies",
    "functioneel altijd actief",
    // e.g. "ik accepteer geen cookies", "nee, ik wil geen optimale cookies"
    /^(nee,?\s+)?(ik\s+)?(wil|accepteer|aanvaard)\s+geen\s+(\S+\s+)?cookies$/is
  ];
  var REJECT_PATTERNS_FRENCH = [
    // refuser / rejeter / interdire / décliner (reject verbs, any position)
    // "refuser et s'abonner" / "refuser et payer" are excluded via BUTTON_NEVER_MATCH_PATTERNS
    // skip English prose such as "cookies that you can refuse"
    /^(?!.*\bcan refuse\b)(.*\s)?(refus|rejet|rejeter|interdire|interdis|déclin|declin)/is,
    // only necessary / essential / technical / functional
    /^(?=.*(cookie|nécessaire|essentiel|techni|requis|obligatoire|fonctionnel|indispensable|sdk)).*(uniquement|seulement|indispensable|strictement nécessaire|que les cookies\s+(nécessaires|techniques|essentiels|indispensables|fonctionnels))/is,
    // continue/proceed without accepting; refuse everything; disable purposes
    /(sans accepter|ne pas accepter|je naccepte rien|je désactive)/is,
    // "non" / "non, merci"
    /^non(,?\s+merci\.?)?$/is
  ];
  var REJECT_PATTERNS_GERMAN = [
    // "... ablehnen" / "ablehnen ..." (reject/decline). Exclude the settings-list phrase "einstell(ungen|en) oder ablehnen".
    /^(?!einstell(ungen|en) oder ablehnen$).*ablehnen/is,
    // verweigern / verweigere / verweigert (refuse)
    /verweiger/is,
    // essential/necessary/functional-only variants (accepting only necessary → reject)
    /^\s*(nur|ausschließlich|lediglich|weiter\s+mit|mit|akzeptiere?n?|unbedingt|es\s+werden\s+nur)?\s*(technisch\s+)?(notwendige?[nrs]?|essenzielle?[nrs]?|essentielle?[nrs]?|erforderliche?[nrs]?|funktionale?[nrs]?|funktionelle?[nrs]?|wesentliche?[nrs]?)\s*(cookies?|technologien|funktionscookies|dienste)?\s*(akzeptieren|erlauben|zulassen|verwenden|annehmen|setzen|speichern|zustimmen|auswählen)?\.?\s*$/is,
    // continue without consent
    /(^|\s)(ohne\s+(zu\s+)?(einwilligung|zustimmung|einverständnis|annahme|annehmen|akzeptanz|akzeptieren)|(weiter|fortfahren)\s+ohne)/is,
    // negations / refusals not covered by the regexes above
    "nein, danke",
    "nein, bitte nicht",
    "nein, ich stimme nicht zu",
    "nicht zustimmen",
    "nicht einverstanden",
    "ich lehne ab",
    "mit erforderlichen einstellungen fortfahren",
    "mit erforderlichen cookies fortfahren",
    "mit notwendigen fortfahren"
  ];
  var REJECT_PATTERNS_ITALIAN = [
    // rifiuta / rifiutare / rifiuto / nega / negare / blocca (reject verbs)
    /(^|\s)(rifiut|neg(a|are|hi)|blocca i cookie)/is,
    // accept/use only necessary / essential / technical
    /^\s*(accetta|accettare|usa|installa|consenti|chiudi e prosegui( solo con)?)?\s*(solo|soltanto|unicamente)?\s*(i\s+|gli\s+)?(cookies?\s+)?(strettamente\s+)?(necessari|necessary|essenziali|tecnici|di navigazione)\s*(necessari|tecnici|essenziali)?\s*$/is,
    // continue without accepting
    /continua(re)? senza accettare/is,
    "non accetto"
  ];
  var REJECT_PATTERNS_BRAZILIAN_PORTUGUESE = [
    // (deny)
    /^\s*(rejeitar|recusar|desativar|bloquear|negar|não\s*aceito|não \s*aceitar)\s*$/is,
    // (proceed) (without accepting)
    /^\s*(continuar|prosseguir|seguir)\s*(sem\s*aceitar)\s*$/is,
    // (deny) (everything) (optional)
    /^\s*(rejeitar|recusar|desativar|bloquear|negar|não\s*aceito|não \s*aceitar)\s*(tudo|o)?\s*(opcional|(não[-\s](essencial|funcional|obrigatório|necessário)))?\s*$/is,
    // (deny) (all) (the) (optional) (cookies)
    /^\s*(rejeitar|recusar|desativar|bloquear|negar|não\s*aceito|não \s*aceitar)\s*(todos)?\s*(os)?\s*(cookies)?\s*(opcionais|(não[-\s](essenciais|funcionais|obrigatórios|necessários)))?\s*$/is,
    // (accept) (only) (the) (essential)
    /^\s*(aceitar|utilizar)?\s*(apenas|somente|só)?\s*(o)?\s*(essencial|funcional|obrigatório|necessário)\s*$/is,
    // (accept) (only) (the) (essential) (cookies)
    /^\s*(aceitar|utilizar)?\s*(apenas|somente|só)?\s*(os)?\s*(cookies)?\s*(essenciais|funcionais|obrigatórios|necessários)\s*$/is
  ];
  var REJECT_PATTERNS_SPANISH = [
    // rechazar / denegar / declinar / negar (reject verbs, any position)
    // "rechazar y pagar" / "rechazar y suscribirse" are excluded via BUTTON_NEVER_MATCH_PATTERNS
    /(^|\s)(rechaz|recház|deneg|negar|declin)/is,
    // accept/allow/use (only) necessary / essential / technical / functional / own
    /^\s*(aceptar?|acepta|permitir|permite|usar|utilizar)?\s*(solo|sólo|només|únicamente)?\s*(las?\s+|los\s+)?(cookies?\s+)?(estrictamente\s+)?(necesari\w*|esencial\w*|técnic\w*|obligatori\w*|funcional\w*|propias)\s*$/is,
    // "solo/sólo/no, sólo ... necessary/essential"
    /^(no,?\s+)?(solo|sólo|només)\s+(usar\s+|las?\s+|los\s+|lo\s+)?.{0,20}(necesari|esencial|estrictamente)/is,
    // refusals / opt-outs
    /^(no acept|no consentir|no permitir|no estoy de acuerdo|no,? gracias|sin consentimiento|continuar sin aceptar|prefiero rechazarlas|descartar todas)/is,
    "acceptar nom\xE9s les necess\xE0ries",
    "nom\xE9s sutilitzen cookies quan \xE9s necessari",
    "pulsa aqu\xED para desactivar las cookies opcionales"
  ];
  var REJECT_PATTERNS_SWEDISH = [
    // avvisa / avböj / neka / förneka (reject verbs)
    /(^|\s)(avvisa|avböj|neka|nekar|förneka)/is,
    // (allow/accept/use) only necessary cookies/kakor
    /(bara|endast|enbart)\s+nödvändig/is,
    // "godkänn/acceptera/använd/tillåt (bara/endast/enbart) nödvändiga (cookies/kakor)"
    /^(ok,?\s+|nej,?\s+)?(jag\s+)?(godkänn\w*|godta|acceptera\w*|använd\w*|tillåt|spara)?\s*(bara|endast|enbart)?\s*(strikt\s+)?nödvändiga?t?( (cookies|kakor|tjänster))?\.?$/is,
    // continue without accepting
    /fortsätt utan att (acceptera|godkänna)/is,
    /strikt nödvändig/is,
    "till\xE5t inte cookies",
    "jag accepterar endast grundl\xE4ggande kakor"
  ];
  var REJECT_PATTERNS_DANISH = [
    // afvis / afvis alle / afvis alle cookies (reject verbs)
    /(^|\s)afvis/is,
    // (accept/allow) only necessary cookies
    /^(accepter\s+|tillad\s+)?(kun|udelukkende)\s+(strengt\s+)?nødvendige( cookies)?$/is,
    // "nej tak" (no thanks), e.g. "du kan sige nej tak til statistik"
    /^nej(,?\s+tak)?$/is,
    /^(du kan )?(sige )?nej,? tak til \S+$/is
  ];
  var REJECT_PATTERNS_NORWEGIAN = [
    // avvis / avslå (reject verbs), not the status "avslått" (rejected)
    /(^|\s)(avvis|avslå)(\s|$)/is,
    // (accept/allow/use) only necessary cookies
    /^(godta\s+|tillat\s+|aksepter\s+|bruk\s+)?(kun|bare)\s+(strengt\s+)?nødvendige( (informasjonskapsler|cookies))?$/is,
    // "nei" / "nei takk" (no thanks)
    /^nei(,?\s+takk)?$/is
  ];
  var REJECT_PATTERNS_CATALAN = [
    /(^|\s)rebutj/is,
    "no accepto",
    "no, gr\xE0cies",
    // accept/allow/use (only) necessary / essential / technical / functional
    /^\s*(acceptar?|accepto|permetre|utilitzar|usar)?\s*(només|sols|únicament)?\s*(les\s+|els\s+)?(galetes\s+|cookies?\s+)?(estrictament\s+)?(necess[aà]ri\w*|essencial\w*|tècni\w*|funcional\w*|imprescindibles)\s*$/is,
    // continue without accepting
    /^continuar sense acceptar/is
  ];
  var REJECT_PATTERNS_GALICIAN = [/(^|\s)rexeitar/is];
  var REJECT_PATTERNS_BASQUE = [/(^|\s)(baztertu|ukatu)/is];
  var REJECT_PATTERNS_PORTUGUESE = [/^aceitar apenas cookies essenciais\.$/];
  var REJECT_PATTERNS_CZECH = ["povolit pouze nezbytn\xE9 cookie"];
  var REJECT_PATTERNS_POLISH = [
    // odrzuć / odrzucam / odmawiam / rezygnuję / blokuj wszystkie (reject verbs)
    /odrzu(ć|cam|cenie|cać|canie|cić)|odmaw|odmowa|odmów|rezygnuj|blokuj wszystk/is,
    // (accept) only necessary / required
    /(^|\s)tylko\s+(bezwzględnie\s+)?(niezbędn\w*|wymagan\w*|konieczne)/is,
    /(akceptuj|akceptuję|zaakceptuj|zatwierdź|potwierdzam|zezwól)\s+(tylko\s+)?(na\s+)?(niezbędn\w*|wymagan\w*|konieczne)/is,
    /korzystaj wyłącznie z niezbędn/is,
    // continue without accepting / consent
    /kontynuuj bez (akceptacj|akceptowani|wyrażania zgody)/is,
    // refusals
    /nie (akceptuję|zgadzam|wyrażam zgody|wyrażaj zgody|zezwalaj|potwierdzam)/is,
    /^nie(,?\s+(dziękuję|nie zgadzam.*))?$/is,
    "niezb\u0119dne",
    "niezb\u0119dne pliki cookie",
    /^funkcjonalne pliki cookie \(wymagane\)$/
  ];
  var REJECT_PATTERNS_RUSSIAN = [
    // отклонить / отказаться / запретить (reject verbs); \w does not match Cyrillic
    /(^|\s)(отклон[а-яё]*|отказ[а-яё]*|откаж[а-яё]*|запрет[а-яё]*|запрещ[а-яё]*)/is,
    // "только необходимые (файлы cookie)" / "принять только необходимые куки"
    /^\s*(принять\s+|принима[а-яё]+\s+|разрешить\s+|использовать\s+|оставить\s+)?(только|лишь)\s+(строго\s+)?(необходим[а-яё]+|нужн[а-яё]+|обязательн[а-яё]+|техническ[а-яё]+|функциональн[а-яё]+)(\s+файл[а-яё]*)?([\s-]+(cookie|куки)([\s-]+файл[а-яё]*)?)?\s*$/is,
    // refusals: "не принимаю", "не согласен", "не соглашаюсь", "нет, спасибо"
    /^\s*(не\s+(принима[а-яё]+|соглас[а-яё]+|разреша[а-яё]+|хочу)|нет(,?\s+спасибо)?)\s*$/is
  ];
  var REJECT_PATTERNS_TURKISH = ["reddet", "\xE7erezleri reddet"];
  var REJECT_PATTERNS_INDONESIAN = ["tolak cookie"];
  var REJECT_PATTERNS_JAPANESE = [
    // e.g. "拒否", "すべて拒否", "全て拒否する", "cookieを拒否"
    /^(すべて|全て)?\s?(の)?(cookie|クッキー)?\s?(を)?拒否(する)?$/i,
    "\u540C\u610F\u3057\u306A\u3044",
    "\u540C\u610F\u3057\u307E\u305B\u3093"
  ];
  var REJECT_PATTERNS = [
    ...REJECT_PATTERNS_ENGLISH,
    ...REJECT_PATTERNS_DUTCH,
    ...REJECT_PATTERNS_FRENCH,
    ...REJECT_PATTERNS_GERMAN,
    ...REJECT_PATTERNS_ITALIAN,
    ...REJECT_PATTERNS_BRAZILIAN_PORTUGUESE,
    ...REJECT_PATTERNS_SPANISH,
    ...REJECT_PATTERNS_SWEDISH,
    ...REJECT_PATTERNS_DANISH,
    ...REJECT_PATTERNS_NORWEGIAN,
    ...REJECT_PATTERNS_CATALAN,
    ...REJECT_PATTERNS_GALICIAN,
    ...REJECT_PATTERNS_BASQUE,
    ...REJECT_PATTERNS_PORTUGUESE,
    ...REJECT_PATTERNS_CZECH,
    ...REJECT_PATTERNS_POLISH,
    ...REJECT_PATTERNS_RUSSIAN,
    ...REJECT_PATTERNS_TURKISH,
    ...REJECT_PATTERNS_INDONESIAN,
    ...REJECT_PATTERNS_JAPANESE
  ];
  var BUTTON_NEVER_MATCH_PATTERNS = [
    // text of several concatenated buttons, or popup body text rather than a button
    /accept all.*reject all|reject all.*accept all/is,
    /^.{300,}$/s,
    /pay|subscribe/is,
    /abonneer/is,
    /abonnier/is,
    /abonner/is,
    /abbonati/is,
    /iscriviti/is,
    /abbonare/is,
    /iscrivere/is,
    /sostienici/is,
    /suscribir/is,
    // Spanish (ES)
    /^abandonar este sitio$/,
    /suscribo/,
    /^accede gratis con cookies publicitarias$/,
    /pagar/,
    /suscríbete/,
    /sin cookies .{0,10}euro/s,
    // Polish (PL)
    /subskrybuj/,
    // Russian (RU): paywall/subscription wording, e.g. "отказаться от подписки", "оплатить"
    /подписатьс|подписк|оплатит|оплачива/is
  ];
  var SETTINGS_PATTERNS = [
    // Multilingual "open customization" patterns: a customization verb next to a
    // cookie/preference/settings/options/details/purposes noun (both word orders).
    // The negative lookahead avoids policy links and save/confirm/accept phrases (those are accept/acknowledge/other).
    /^(?!.*\b(policy|policies|notice|statement|impressum|\w*richtlinie\w*|\w*erklärung|page|beschrijving|privacy$|beleid|politique|política|polityk|speichern|guardar|opslaan|zapisz|enregistrer|sauvegarder|bevestig|bestätig|confirm|save|submit|akzeptier|accept|zaakcept)\b)(customi[sz]e|manage|adjust|configure|personali[sz]e|let me choose|edit|change|set|select|view|see|review|update|open|show|choose|anpassen|verwalten|konfigurieren|bearbeiten|öffnen|anzeigen|einblenden|einstellen|festlegen|auswählen|wählen|aanpassen|beheren|instellen|wijzig\w*|personaliseren|personaliseer|kies|bekijk|toon|personnaliser|paramétrer|gérer|configurer|choisir|afficher|définir|modifier|configurar|personalizar|gestionar|administrar|ajustar|seleccionar|modificar|establecer|dostosuj|zarządzaj|personalizuj|ustaw\w*|zmień|pokaż|wybierz)(?!\w).{0,20}(cookies?|preferences?|settings?|options?|choices?|controls?|details?|purposes?|services?|consent|einstellung\w*|optionen|präferenzen|einzelheiten|zwecke|dienste|datenschutz\w*|auswahl|voorkeur\w*|instelling\w*|opties|diensten|préférences|paramètres|réglages|choix|détails|finalités|témoins|preferencia\w*|opciones|ajustes|configuraci\w*|detalles|servicios|elección|preferencj\w*|ustawie\w*|opcje|szczegół\w*|cele|galetes)\b/is,
    /^(?!.*\b(policy|policies|notice|statement|impressum|\w*richtlinie\w*|\w*erklärung|page|beschrijving|privacy$|beleid|politique|política|polityk|speichern|guardar|opslaan|zapisz|enregistrer|sauvegarder|bevestig|bestätig|confirm|save|submit|akzeptier|accept|zaakcept)\b)(cookies?|preferences?|settings?|options?|choices?|controls?|details?|purposes?|services?|consent|einstellung\w*|optionen|präferenzen|einzelheiten|zwecke|dienste|datenschutz\w*|auswahl|voorkeur\w*|instelling\w*|opties|diensten|préférences|paramètres|réglages|choix|détails|finalités|témoins|preferencia\w*|opciones|ajustes|configuraci\w*|detalles|servicios|elección|preferencj\w*|ustawie\w*|opcje|szczegół\w*|cele|galetes)\b(.{0,14}\W)?(customi[sz]e|manage|adjust|configure|personali[sz]e|let me choose|edit|change|set|select|view|see|review|update|open|show|choose|anpassen|verwalten|konfigurieren|bearbeiten|öffnen|anzeigen|einblenden|einstellen|festlegen|auswählen|wählen|aanpassen|beheren|instellen|wijzig\w*|personaliseren|personaliseer|kies|bekijk|toon|personnaliser|paramétrer|gérer|configurer|choisir|afficher|définir|modifier|configurar|personalizar|gestionar|administrar|ajustar|seleccionar|modificar|establecer|dostosuj|zarządzaj|personalizuj|ustaw\w*|zmień|pokaż|wybierz)(?!\w)/is,
    // settings noun + cookies, e.g. "paramètres des cookies", "ajustes de cookies", "instellingen voor cookies en diensten"
    /^(paramètres|préférences|réglages|ajustes|opciones|elección|preferencias|instellingen)( (de|des|les|voor|van))? cookies( (et des|en|y) (services|diensten|servicios))?$/is,
    "settings",
    "preferences",
    /customi(s|z)e/is,
    "more options",
    /^(?!.*\b(polic(y|ies)|notice|statement|privacy page)\b).*(manage|configure) (my|your) (preferences|choices|cookies)/is,
    /(cookie )?preference center/is,
    "configure",
    "cookie manager",
    "cookie preference",
    "let me choose",
    "cookieconsent preferences",
    /privacy choices/is,
    /^(privacy|cookie|custom) settings$/is,
    /^cookies? (settings|preferences|setting)(?!.*\bdo not\b)/is,
    /^(?!.*\b(polic(y|ies)|notice|statement|privacy page)\b).*(manage|customize|customise|opt-out|edit).*(cookies|preferences|settings|options)/is,
    "cookie consent options",
    "privacy controls",
    // German
    "einstellungen",
    // Spanish (ES)
    /^(configurar|configuración|administrar)$/,
    /^(gestionar|ver|establecer) preferencias$/,
    "ajustes",
    "centro de preferencias",
    "configura",
    /^configuración( de( las)?)? cookies?( y servicios)?$/,
    /^configurar\.\.\.$/,
    "configurarlas",
    "detalles",
    "gestiona tus preferencias",
    /^gestionar ?(las?|mis)? ?(configuración|preferencias)?(( de)? cookies?)?$/,
    "gesti\xF3n cookies",
    "gesti\xF3n de cookies",
    "mis preferencias",
    "mostrar detalles",
    "mostrar los prop\xF3sitos",
    "m\xE1s opciones",
    "no, ajustar",
    "obtener m\xE1s informaci\xF3n y configuraci\xF3n",
    "opciones de gesti\xF3n",
    "panel de configuraci\xF3n de cookies",
    "personalice",
    "personalizar",
    "preferencias de privacidad",
    "preferencias",
    "quiero configurarlas",
    "saber m\xE1s y personalizar",
    "seleccionar fines individuales",
    // Catalan (CA)
    "configura-les",
    "personalitza",
    "veure prefer\xE8ncies",
    /^(configuració|personalitzar|preferències|més opcions)$/,
    /^(configuració|preferències|opcions)( de( les)?)? (galetes|cookies?)$/is,
    /^(gestiona|gestionar|configura|configurar|personalitza|personalitzar|veure|establir|administrar)( les)?( meves)? (preferències|galetes|cookies?|opcions)$/is,
    // Galician (GL)
    "xestionar preferencias",
    // Basque (EU)
    /^(konfigurazioa|konfiguratu)$/,
    // Portuguese (PT)
    "gerenciar cookies",
    // French (FR)
    "param\xE9trage des cookies",
    "param\xE9trer",
    "personnaliser",
    "param\xE8tres",
    "pr\xE9f\xE9rences",
    "r\xE9glages",
    "d\xE9tails",
    "gestion des cookies",
    /^gérer (les |mes )?cookies$/,
    "je choisis",
    "voir les pr\xE9f\xE9rences",
    // German (DE)
    "abschnitt einzelheiten",
    "cookie-details",
    /^datenschutz-?einstellungen$/,
    "cookie-einstellungen",
    /^einstell(ungen|en) oder ablehnen$/,
    "ausw\xE4hlen",
    /^einstellungen (anpassen|ansehen|verwalten|ändern)$/,
    "erweiterte einstellungen",
    "individuelle datenschutz-pr\xE4ferenzen",
    "individuelle datenschutzeinstellungen",
    "konfigurieren",
    "mehr optionen",
    "pr\xE4ferenzen",
    "individuelle einstellungen",
    "privatsph\xE4re einstellungen",
    // Dutch (NL)
    /^(aan|an)passen$/,
    /^cookie[- ]instellingen$/,
    "cookiestatement instellingen",
    /^details (tonen|weergeven)$/,
    "instellingen",
    "meer opties",
    "zelf instellen",
    // Czech (CS)
    "podrobn\xE9 nastaven\xED",
    // Polish (PL)
    // examples:
    //  dostosuj pliki cookie (adjust cookies)
    //  zarządzaj plikami cookie (manage cookies)
    /^(dostosuj|s?personalizuj|chcę dostosować|zarządzaj) ?(moje|moimi)? ?(ustawieniami|preferencjami)? ?(zgody|wybory|(plik(i|ami|ów))? cookies?)?$/,
    /^(preferencje|zarządzaj preferencjami)$/,
    /^(ustawienia|zmień ustawienia|zmiana ustawień|zarządzaj opcjami)$/,
    "centrum preferencji",
    /^chcę dokonać ustawień cookies\.$/,
    "dostosuj wyb\xF3r",
    "edytuj ustawienia",
    "konfiguracja zg\xF3d",
    "otw\xF3rz ustawienia",
    "personalizacja",
    "poka\u017C cele",
    "poka\u017C szczeg\xF3\u0142y",
    "szczeg\xF3\u0142y",
    "pozw\xF3l mi wybra\u0107",
    /^przejdź do ustawień plików cookies\.$/,
    "przejd\u017A do ustawie\u0144 prywatno\u015Bci",
    "przejd\u017A do ustawie\u0144",
    "skonfiguruj",
    "ustaw swoje wybory",
    "ustawienia ciasteczek",
    "ustawienia prywatno\u015Bci",
    "ustawienia zaawansowane",
    "ustawienia zgody",
    /^ustawienia(ch)?( plików)? cookies?$/,
    "ustawieniach",
    "ustawie\u0144 zaawansowanych",
    "wi\u0119cej opcji",
    "wi\u0119cej ustawie\u0144",
    /^wybierz, jakie pliki cookies chcesz zaakceptować\.$/,
    "zaawansowane",
    "zarz\u0105dzaj zgodami dotycz\u0105cymi plik\xF3w cookies",
    "zarz\u0105dzaj zgodami",
    "zarz\u0105dzania zgodami",
    "zarz\u0105dzanie opcjami",
    "zarz\u0105dzanie preferencjami",
    "zarz\u0105dzanie ustawieniami plik\xF3w cookie",
    "zmieniam ustawienia",
    "zmieniam zgody",
    "zmie\u0144 swoje preferencje",
    /^zmień ustawienia( plików)? cookies?$/,
    "zmie\u0144 zgody",
    "zobacz preferencje",
    // Russian (RU)
    // e.g. "настроить", "настройки куки", "параметры конфиденциальности"; \w does not match Cyrillic
    /^\s*((мои|моими|свои|своими)\s+)?(настро[а-яё]+|парамет[а-яё]+|предпочтени[а-яё]+)([\s-]+(мои|моими|свои|своими|файл[а-яё]*|cookie|куки[а-яё]*|конфиденциальност[а-яё]*|согласи[а-яё]*|приватност[а-яё]*))*\s*$/is,
    // e.g. "управление файлами cookie", "изменить настройки", "выбрать категории"
    // "подробн" is deliberately excluded: it would also match the "подробнее о cookie" policy link
    /^\s*(управл[а-яё]+|измен[а-яё]+|выбрать|персонализ[а-яё]+|расширенн[а-яё]+|индивидуальн[а-яё]+)[\s-]+.{0,20}(настройк[а-яё]*|парамет[а-яё]*|предпочтени[а-яё]*|категори[а-яё]*|файл[а-яё]*|cookie|куки[а-яё]*|согласи[а-яё]*|конфиденциальност[а-яё]*)\s*$/is,
    "\u043F\u043E\u0434\u0440\u043E\u0431\u043D\u044B\u0435 \u043D\u0430\u0441\u0442\u0440\u043E\u0439\u043A\u0438",
    // Italian (IT)
    "personalizza cookie",
    // English (EN)
    "advanced settings",
    "consent settings",
    /^details (anzeigen|zeigen|section)$/,
    "no, adjust",
    "personalize",
    "plus doptions",
    "privacy manager"
  ];
  var ACCEPT_PATTERNS = [
    // EN accept/agree/allow/consent/enable (+ all/cookies/selection/continue/close/proceed...).
    // The negative lookahead avoids essential-only/reject wording (those are reject).
    /^(?!.*\b(essential|necessary|required|functional|minimal|reject|deny|refuse|decline|only|dismiss)\b)(?!.*all cookies continue)(yes,?\s+)?(i\s+)?(accept|agree|allow|consent|enable)(\s+(to\s+)?(all( cookies)?|cookies|selection|selected( cookies)?|everything|recommended( cookies| settings)?|optional( cookies)?|additional cookies|analytics cookies))?(\s+(and\s+)?(continue|close|proceed|save))?\s*$/is,
    /^continue (and accept|using cookies|with (all|recommended cookies|cookies))$/is,
    // DE accept verbs
    /^(alle[sn]?\s+|allem\s+|ich\s+|cookies\s+|ausgewählte\s+|webanalyse\s+)?(cookies?\s+)?(akzeptieren|annehmen|zustimmen|zulassen|erlauben|einwilligen|aktivieren|auswählen)(\s+(und\s+)?(weiter|schlie(ß|ss)en))?\s*$/is,
    /^((meine\s+)?auswahl|alle)\s+(bestätigen|akzeptieren|auswählen)$/is,
    /^(alle[nm]?\s+)?(zustimmen|einverstanden|einwilligung|zustimmung)$/is,
    /^ich (bin einverstanden|akzeptiere( alle)?|stimme zu)$/is,
    // NL accept verbs
    /^(ja,?\s+)?(alle[s]?\s+|ik\s+)?(cookies?\s+)?(accepteer|accepteren|toestaan|aanvaard|aanvaarden|ga akkoord|akkoord)(\s+((en\s+)?sluiten|en\s+(doorgaan|verdergaan)|cookies|alle))?\s*$/is,
    /^(selectie (accepteren|toestaan)|accepteer (selectie|alle)|alle (toestaan|accepteren|aanvaarden)|ja, (dat is prima|prima|alles toestaan|accepteren|ik accepteer cookies|ik ga akkoord)|is goed)$/is,
    // FR accept verbs
    /^(oui,?\s+)?(je\s+)?(tout\s+)?(accepter|jaccepte|autoriser)(\s+(tout|tous les (cookies|témoins)|les (cookies|témoins)|la sélection|et (continuer|fermer|poursuivre)))?\s*$/is,
    /^(oui, (jaccepte|je suis daccord)|jaccepte (les cookies|lutilisation de cookies)|accepter (continuer|et poursuivre)|continuer et accepter|fermer et accepter)$/is,
    // ES/CA accept verbs
    /^(sí,?\s+|si,?\s+)?(aceptar|acepta|permitir|permitirlas|consentir|estoy de acuerdo|de acuerdo|estic dacord)(\s+(todo|todas( las cookies)?|cookies|la selección|selección|y (cerrar|continuar|seguir( leyendo)?|leer gratis)))?\s*$/is,
    // save / submit selection / preferences (accept semantics; acknowledge catches "guardar configuración/selección" first)
    /^(save|store|submit|guardar|sauvegarder|zapisz|opslaan|bewaar)\b.{0,25}(preference|setting|selection|choice|mes choix|cookie|voorkeur|keuze|ustawien|zgod)/is,
    /^((meine\s+)?auswahl|einstellungen|einwilligung|voorkeuren|instellingen|selectie|keuzes)\s+(speichern|opslaan)$/is,
    /i (accept|allow)( all)?/is,
    "yes",
    /accept all above/is,
    "close and accept",
    /(^|\s)accept all$/is,
    "im ok with that",
    // Spanish (ES)
    /^acept(ar|o)( cookies)?$/,
    /^acept(o|ar) todas las cookies$/,
    "aceptar cookies opcionales",
    "aceptar gratis",
    "aceptar las cookies",
    "aceptar todas cookies",
    "aceptar todas y cerrar",
    "aceptar todas y continuar",
    "aceptar todo y cerrar",
    /^aceptar y (continuar|seguir|navegar)( gratis)?$/,
    "aceptar y seguir navegando",
    "aceptarlas todas",
    "guardar preferencias",
    "ok, las acepto",
    /^s[íi], acepto todas las cookies$/,
    /^s[íi], acepto$/,
    /^s[íi], estoy de acuerdo$/,
    "x aceptar y cerrar",
    // Catalan (CA)
    /^accept(ar|o)( cookies)?$/,
    "accepta",
    "accepta totes les cookies",
    "accepta-ho tot",
    // FR/CA "d'accord" / "d'acord"
    /^dac?cord( fermer)?$/is,
    "accepta-les totes",
    "acceptar galetes",
    "acceptar i tancar",
    "acceptar tot",
    "acepta-les totes",
    "permet-les totes",
    "permetre totes les cookies",
    "permetre la selecci\xF3",
    // Basque (EU)
    /^denak? onartu$/,
    /^onartu \(cookie\)$/,
    "onartu cookieak",
    "onartu",
    // Portuguese (PT)
    "aceitar cookies",
    "aceitar",
    "de acordo",
    // French (FR)
    /^accepter (tout|tous les cookies|fermer)$/,
    // German (DE)
    /^(alles akzeptieren|alle zulassen|auswahl erlauben|cookies zulassen|einverstanden|einwilligung|zustimmen|zustimmung)$/,
    // Dutch (NL)
    /^(accepteer (alles|alle cookies)|alles (accepteren|toestaan)|alle cookies (accepteren|toestaan))$/,
    // Czech (CS)
    "souhlas\xEDm",
    // Polish (PL)
    // examples:
    //  akceptuj cookies (Accept cookies)
    //  akceptuj wszystkie pliki cookie (Accept all cookies)
    /^(zaakceptuj|akceptuj[eę]|akceptuj) ?(wszystkie|wszystko)?( pliki)? ?(zgody|ciasteczka|cookies?)?$/,
    "akceptowanie plik\xF3w cookie",
    "akceptuj wybrane",
    "akceptuj i zamknij",
    "akceptuj wszystkie i przejd\u017A do serwisu",
    "akceptuj\u0119 i przechodz\u0119 do serwisu",
    "akceptuj\u0119 polityk\u0119 plik\xF3w cookies i przechodz\u0119 do strony",
    "akceptuj\u0119 ustawienia cookies",
    "akceptuj\u0119 wszystkie i korzystam z us\u0142ug",
    "akceptuj\u0119!",
    "ok, zgadzam si\u0119",
    "potwierdzam wszystkie",
    "tak",
    "tak, zgadzam si\u0119 na wszystkie pliki cookie",
    "tak, zgadzam si\u0119",
    "wyra\u017A zgod\u0119 na wszystko",
    "wyra\u017Cam zgod\u0119 na wszystkie",
    "wyra\u017Cam zgod\u0119",
    "w\u0142\u0105cz wszystkie ciasteczka",
    "zaakceptuj i kontynuuj",
    "zaakceptuj i zamknij",
    "zaakceptuj wszystkie i przejd\u017A do serwisu",
    "zaakceptuj wszystkie zgody i wejd\u017A do serwisu",
    "zaakceptuj wszystkie zgody i zapisz",
    "zatwierd\u017A",
    "zezwolenie na wszystkie",
    "zezw\xF3l na wszystkie ciasteczka",
    "zezw\xF3l na wszystkie cookies",
    "zezw\xF3l na wszystkie pliki cookies",
    "zezw\xF3l na wszystkie",
    "zezw\xF3l na wyb\xF3r",
    "zezw\xF3l",
    "zgadzam si\u0119 na wszystkie",
    "zgadzam si\u0119",
    "zgoda na wszystkie",
    "zgoda",
    "zaakceptuj wybrane",
    "zezw\xF3l na wybrane",
    "zgoda na wybrane",
    // Russian (RU)
    // e.g. "принять всё", "принять все файлы cookie", "разрешить куки", "я согласен"
    // confirm/save wording (подтвердить, сохранить) is acknowledge, not accept
    /^\s*(да,?\s+)?(я\s+)?(принять|принима[а-яё]+|соглас[а-яё]+|разрешить|разреша[а-яё]+)(\s+(вс[её]|(все\s+)?(файлы[\s-]+)?(cookie|куки)([\s-]+файл[а-яё]*)?|выбранные|выбор))?\s*$/is,
    "\u043F\u0440\u0438\u043D\u044F\u0442\u044C",
    // Turkish (TR)
    "kabul et",
    // Italian (IT)
    "accetta",
    "accetta tutti i cookie",
    "accetto tutti i cookie"
  ];
  var ACKNOWLEDGE_PATTERNS = [
    // close / dismiss the banner/dialog/message (multilingual). The negative lookahead avoids
    // accept/save phrases (e.g. "agree and close", "akkoord en sluiten", "speichern schließen").
    /^(?!.*\b(accept\w*|accepter|accepteer|accepteren|agree|allow|akkoord|aanvaard\w*|zustimm\w*|annehm\w*|akzeptier\w*|aceptar|acepta|permit\w*|consent\w*|einverstanden|zezw\w*|zgadzam|zgoda|guardar|opslaan|enregistrer|speicher\w*|zapisz)\b)(x\s+|nee,?\s+)?(close|dismiss|schlie(ß|ss)en|sluiten|afsluiten|fermer|cerrar|tanca|beenden|masquer|zamknij|ukryj)( (this|the|ce|le|el|de|des|het|la|een)?\s*(banner|bandeau|banier|bar|dialog|dialogue|disclaimer|window|okno|melding|message|notification|informa\w*|notificaci\w*|cookie\w*|bannière|rgpd|gdpr|hier|des cookies|de cookies|x))*\.?\s*$/is,
    // "ok" / "okay" / "oké" (optionally followed by a short acknowledgement)
    /^(ok|okay|oké|okey|k)([ .!,]*)(got it|verstanden|compris|rozumiem|thanks|gracias|ik begrijp( dat| het)?|continue to website|pour moi|fermer)?[ .!]*$/is,
    // "understood" / "got it" / "that's ok" (multilingual)
    /^(i understand|understood|got it|thats (ok|fine|okay)|alright|alles klar|in ordnung|verstanden|begrepen|jai compris|je comprends|compris|ik begrijp het|ik snap het|entendido|c(e)?st ok pour moi)[ !.,]*(merci|bedankt|dismiss this banner)?[ !.]*$/is,
    // confirm
    /^(confirm|bestätigen|bevestigen|confirmar|potwierdź)[ !.]*$/is,
    // neutral "continue" without accept/reject wording
    /^(continuer|poursuivre|doorgaan|ga verder)$/is,
    // e.g. "continue browsing", "continue to browse the site"
    /^continue (to )?brows(e|ing)( (the|our) (site|website))?$/is,
    // German "hide", e.g. "ausblenden", "hinweis ausblenden", "ok. ausblenden."
    /^(ok[ .!,]*)?(hinweis )?ausblenden[ .!]*$/is,
    "continue",
    "x",
    /^got it!?$/,
    "acknowledge",
    // notice-only banner with no choice
    "i agree, dismiss this banner",
    /^close (banner|cookie notification)$/is,
    /understood$/is,
    "confirm my choices",
    // German (DE)
    "speichern schlie\xDFen",
    // Spanish (ES)
    /^.?( lo)?(entendido|entiendo).?$/s,
    "aceptar seleccionadas",
    "continuar",
    "guardar configuraci\xF3n",
    "guardar selecci\xF3n",
    "guardar y cerrar",
    "ir al contenido principal",
    "seguir",
    "vale",
    "\xA1vamos!",
    // Catalan (CA)
    /^(ho )?(entenc|entès|entesos)[ !.]*$/s,
    // Polish (PL)
    "kontynuuj",
    "ok, zrozumia\u0142em",
    /^ok.? rozumiem.?$/s,
    "rozumiem!",
    "rozumiem",
    "rozumiem, nie pokazuj wi\u0119cej",
    "w porz\u0105dku!",
    "w porz\u0105dku",
    /^zamknij informację o( plikach)? cookies$/,
    "zapisz i zamknij",
    // Russian (RU)
    // e.g. "понятно", "всё понятно", "хорошо", "ясно"; \w does not match Cyrillic
    /^\s*(ок|окей|хорошо|ясно|(вс[её]\s+)?(понятно|понял[аи]?))[!.]*\s*$/is,
    // e.g. "закрыть", "закрыть уведомление о cookie"
    /^\s*закрыть([\s-]+(это|эту|баннер|уведомлени[а-яё]*|окно|сообщени[а-яё]*|плашк[а-яё]*|информаци[а-яё]*)){0,2}([\s-]+(о|об)[\s-]+(cookie|куки[а-яё]*|файл[а-яё]*[\s-]+cookie))?\s*$/is,
    // e.g. "больше не показывать", "не показывать снова"
    /^\s*(больше\s+не\s+показывать|не\s+показывать(\s+(снова|больше|это\s+сообщение))?)\s*$/is,
    // e.g. "подтвердить", "подтверждаю выбор", "сохранить настройки", "сохранить и закрыть"
    /^\s*(подтвер(дить|ждаю|ждени[а-яё]*)|сохран(ить|яю|ение))([\s-]+(мой|мои|моё|свой|свои|своё)?[\s-]*(выбор[а-яё]*|настройк[а-яё]*|парамет[а-яё]*|предпочтени[а-яё]*|согласи[а-яё]*))?([\s-]+и[\s-]+(закрыть|продолжить))?\s*$/is,
    "\u043F\u0440\u043E\u0434\u043E\u043B\u0436\u0438\u0442\u044C",
    // Japanese (JA)
    // e.g. "閉じる" (close), "確認" / "確認しました" (confirmed), "了解" (understood), "確認して閉じる";
    // not "同意して閉じる" (agree and close) or the verb "確認する" (confirm)
    /^(確認|了解)(しました)?$|^((確認|了解)して)?閉じる$/
  ];

  // lib/heuristic-negatives.ts
  var COOKIE_CHOICE = [
    "\\b(?:accept|reject|decline|refuse|deny|allow|manage|customi[sz]e)\\b[^\\n]{0,25}\\bcookies?\\b",
    "\\b(?:reject|decline|deny|refuse) all\\b",
    "\\b(?:essential|necessary|required) (?:cookies )?only\\b|\\bonly (?:essential|necessary|required)\\b",
    "\\bcookies? (?:notice|settings|preferences|banner)\\b",
    "\\b(?:alle )?(?:cookies )?ablehnen\\b",
    "\\b(?:alles |alle cookies )?(?:weigeren|weiger|afwijzen)\\b",
    "\\b(?:tout )?refuser\\b",
    "\\brechazar(?: todas?)?\\b",
    "\\brifiuta(?: tutti)?\\b",
    // purposes and vendors as a button or heading line: some consent dialogs ask "under 18?"
    "^[^\\n]{0,40}\\b(?:purposes|finalit\xE9s|prop\xF3sitos|zwecke|doeleinden|finalit\xE0|vendors|fournisseurs|proveedores|leveranciers|fornitori)(?!\\w)[^\\n]{0,10}$",
    "\\b(?:legitimate (?:business )?interest|int\xE9r\xEAt l\xE9gitime|inter\xE9s leg\xEDtimo|berechtigtes interesse|gerechtvaardigd belang|legittimo interesse)\\b",
    "\\b(?:we|nous|wir|wij|noi|nosotros) (?:and|et|und|en|e|y) (?:our|nos|unsere|onze|i nostri|nuestros) (?:\\d+ )?(?:partners|partenaires|partner|socios)\\b"
  ].join("|");
  var BARE_ACCEPT = [
    "\\b(?:accept|allow) all\\b",
    "\\b(?:alle )?(?:cookies )?(?:akzeptieren|erlauben|zulassen)\\b",
    "\\b(?:alles |alle cookies )?(?:accepteren|accepteer|toestaan)\\b",
    "\\b(?:tout )?accepter\\b",
    "\\baceptar(?: todas?)?\\b",
    "\\baccetta(?: tutti)?\\b"
  ].join("|");
  var COOKIE_CHOICE_RE = new RegExp(COOKIE_CHOICE, "im");
  var CONSENT_CHOICE = new RegExp(`${COOKIE_CHOICE}|${BARE_ACCEPT}`, "im");
  var AGE_WORDING = new RegExp(
    [
      "\\b(?:over|above|at least|older than|minimum(?: age of)?)\\s*(?:18|21|eighteen)\\b",
      // "18+" only with age words after it: a bare "18+" also appears in cookie banners on adult sites
      "\\b(?:18|21)\\+?\\s*(?:years?(?: of age| old)?|yo|or older|or over)\\b",
      "\\b(?:you|users?|visitors?) must be (?:over |above |at least )?(?:18|21)\\b",
      "\\b(?:i'?m|i am) (?:over |at least )?(?:18|21)\\b",
      "\\bage (?:verification|confirmation|check|gate|restriction)\\b",
      "\\b(?:adults? only|adult[\\s-](?:oriented|content|material|website)|legal age|age of majority|not a minor|minors)\\b",
      "\\b(?:18|21) jahre\\b",
      "\\balters(?:beschr\xE4nkung|best\xE4tigung|verifikation|pr\xFCfung|kontrolle)",
      "\\b(?:vollj\xE4hrig|f\xFCr erwachsene|minderj\xE4hrige)\\b",
      "\\b(?:18|21) jaar\\b",
      "\\b(?:volwassenen|meerderjarig|minderjarigen|leeftijdsverificatie)\\b",
      "\\b(?:18|21) ans\\b",
      // no \\b before an accented letter: without the u flag it is not a word character
      "\\b(?:pour adultes|v\xE9rification de l.\xE2ge|mineurs)\\b|\xE2ge l\xE9gal\\b",
      "\\b(?:18|21) a\xF1os\\b",
      "\\b(?:mayor(?:es)? de edad|menores de edad|para adultos|verificaci\xF3n de edad)\\b",
      "\\b(?:18|21) anni\\b",
      "\\b(?:maggiorenne|minorenni|per adulti)\\b|\\bverifica dell.et\xE0(?!\\w)"
    ].join("|"),
    "i"
  );
  var DISMISS_LINE = /^\s*(?:(?:yes,? )?(?:i )?(?:ok(?:ay)?|accept|agree|allow|decline|deny|reject|refuse|understand|acknowledge(?: the use of cookies)?|got it|understood)(?: all)?(?:,? (?:and|&) (?:continue|close))?(?: non-essential)?|close|dismiss|akzeptieren|zustimmen|verstanden|einverstanden|ablehnen|accepteren|akkoord|weigeren|accepter|j.accepte|d.accord|refuser|fermer|aceptar|acepto|rechazar|entendido|accetto|accetta|rifiuta|ho capito|chiudi)\s*[!.]?\s*$/im;
  var DISMISS_END = /[.!?]\s+(?:ok(?:ay)?|got it|i agree|accept)\s*!?\s*$|\[\s*[x×]\s*\]|^\s*[x×✕]\s*$/im;
  var NOTICE_CONTROL = new RegExp(`${CONSENT_CHOICE.source}|${DISMISS_LINE.source}|${DISMISS_END.source}`, "im");
  var LEAVE_LINE = /^\s*(?:(?:no,? )?(?:i )?(?:want to )?(?:leave|exit|verlassen|quitter|sortir|salir|abandonar|esci|uscire|verlaten)\b[^\n]{0,30}|no,? take me away|(?:seite|hier) verlassen|(?:i am|i'm|ich bin|ik ben|je suis|soy|sono) (?:under|unter|onder|jonger dan|menor de(?: edad)?|minorenne|mineur)\b[^\n]{0,10})\s*[!.]?\s*$/im;
  var NOTICE_OWN_CONTROL = new RegExp(
    [
      "\\bcookies? verwalten\\b|\\bcookie-?(?:einstellungen|pr\xE4ferenzen)\\b|\\beinstellungen verwalten\\b",
      "\\bconfigura(?:r|ci\xF3n de)(?: las)? cookies\\b|\\bgestionar(?: las)? cookies\\b|\\bpersonalizar(?: las)? cookies\\b",
      "\\bparam[\xE9e]trer(?: les)? cookies\\b|\\bparam[\xE8e]tres(?: des)? cookies\\b|\\bg[\xE9e]rer(?: les)? cookies\\b|\\bpersonnaliser(?: les)? cookies\\b",
      "\\bgestisci(?: i)? cookie|\\bimpostazioni(?: dei| sui)? cookie|\\bpersonalizza(?: i)? cookie",
      "\\bcookie-?instellingen\\b|\\bcookies? beheren\\b|\\bvoorkeuren (?:beheren|aanpassen)\\b",
      "^\\s*(?:customi[sz]e|personnaliser|param\xE9trer|personalizar|configurar|personalizza|anpassen|aanpassen|meer opties|more options|weitere optionen|plus d.options|m\xE1s opciones|altre opzioni|manage (?:options|preferences|choices))\\s*$",
      "^\\s*close (?:this|the) (?:popup|banner|notice|message)\\s*$"
    ].join("|"),
    "im"
  );
  var ageGateUnless = (control) => (text) => !LEAVE_LINE.test(text) && (control.test(text) || NOTICE_OWN_CONTROL.test(text));
  var CONSENT_WORDS = /cookie|consent|track|partner|personali[sz]|advertis|analytic|privacy (?:options|choices|settings|rights)|do not sell|opt[- ]out/i;
  var LEGAL_AGREEMENT = /\b(?:end[- ]user licen[cs]e agreement|licen[cs]e agreement|limitation of liability|governing law|indemnif\w*|hereunder|disclaimers? of warrant\w*|warrant(?:y|ies) of merchantability)\b/i;
  var INVESTOR_GATE = /\beligible\s+counterpart(?:y|ies)\b|\bnot\s+(?:suitable|intended)\s+for\s+retail\s+(?:clients|investors)\b|\bconfirm\w*\s+(?:that\s+)?(?:you\s+are|i\s+am)\s+an?\s+(?:(?:professional|institutional|qualified|accredited|sophisticated)\s+(?:investor|client)|financial\s+(?:intermediary|advis[eo]r))\b/i;
  var DETECT_CONTEXT_NEGATIVES = [
    // popups with their own age confirmation are age gates, even when they mention cookies; a bare accept doesn't count
    // as a cookie choice here, since it may confirm the age
    { name: "age-gate-popup", pattern: AGE_WORDING, unless: ageGateUnless(COOKIE_CHOICE_RE), whole: true, input: "popup" },
    // a license or terms agreement may mention cookies, and its decline button must not be clicked as a cookie reject
    { name: "legal-agreement", pattern: LEGAL_AGREEMENT, whole: true, input: "popup" },
    // an investor eligibility gate may mention cookies, and its decline button must not be clicked as a cookie reject
    { name: "investor-gate", pattern: INVESTOR_GATE, whole: true, input: "popup" },
    // page text next to an age gate, unless a consent control is there too
    { name: "age-gate", pattern: AGE_WORDING, unless: ageGateUnless(NOTICE_CONTROL), lines: [4, 5], input: "page" },
    // terms of use need cookie or consent wording; a privacy-policy-only notice needs that or a button
    {
      name: "terms-without-consent",
      pattern: /\bterms\b|\bconditions\b/i,
      unless: CONSENT_WORDS,
      appliesTo: [BY_CONTINUING_PRIVACY]
    },
    {
      name: "privacy-without-control",
      unless: new RegExp(`${CONSENT_WORDS.source}|${DISMISS_LINE.source}`, "im"),
      appliesTo: [BY_CONTINUING_PRIVACY]
    },
    // a cookie policy link among short footer link lines
    {
      name: "footer-link",
      pattern: /^[^.!?:\n]{0,60}\n[\s]*\n?[^.!?:\s][^.!?:\n]{0,59}(?:\n|$)/,
      unless: NOTICE_CONTROL,
      lines: [0, 2],
      input: "page",
      appliesTo: [USE_OF_COOKIES, VERWENDUNG_VON_COOKIES, UTILISATION_DES_COOKIES, GEBRUIK_VAN_COOKIES]
    },
    {
      name: "acceptez-terms",
      pattern: /conditions (?:générales )?d.utilisation|\bcgu\b/i,
      unless: /cookie|témoin|traceur|partenaire/i,
      appliesTo: [ACCEPTEZ_UTILISATION]
    },
    // the same sentence is also a footer line with only a "learn more" link
    {
      name: "de-cookie-verb-needs-control",
      unless: NOTICE_CONTROL,
      lines: [1, 6],
      input: "page",
      appliesTo: [DE_COOKIE_VERB, DE_THIS_SITE_COOKIES, DE_SITE_USES_COOKIES]
    }
  ];

  // lib/heuristic-literals.ts
  var DETECT_PATTERN_LITERALS = [
    ["accept cookies"],
    ["only necessary cookies"],
    [],
    ["by using "],
    ["by clicking"],
    ["accept all cookies"],
    ["reject non-"],
    ["cookies "],
    [" cookies on your "],
    ["your consent to "],
    ["you agree to "],
    ["blocking some types of cookies"],
    ["powered by usercentrics", "realisiert mit", "realized with"],
    [],
    [" cookie"],
    ["cookie files"],
    ["cookies"],
    ["cookies?"],
    ["we are using cookies"],
    ["use of cookies"],
    ["website uses cookies to enhance your browsing experience"],
    ["cookies"],
    [" technologies"],
    ["such as cookies"],
    ["read more about"],
    ["consent to"],
    ["we and our "],
    ["information"],
    ["store and/or access information"],
    ["personalised ads and content, ad and content measurement"],
    [" and/or access"],
    ["and our partners ask for your consent"],
    ["we and "],
    ["partners", "partnern", "partner", "partenaires", "socios", "vendors", "fornitori"],
    ["partner"],
    ["technolog"],
    ["cookies"],
    ["cookies"],
    ["optional", "additional", "non-essential"],
    ["storing of cookies", "placed cookies"],
    [" on your device", "access and store "],
    ["we use "],
    ["out of "],
    ["out of the sale"],
    ["this website or its third"],
    ["information"],
    ["rights with regard to the sale"],
    ["we use technologies "],
    ["party tools to collect information"],
    ["continue reading with advertising", " tracking"],
    ["consent and continue"],
    ["store or retrieve information on your browser", "stock\xE9es dans votre navigateur ou r\xE9cup\xE9r\xE9es"],
    ["we have cookies"],
    [" your consent at any time", "einwilligung jederzeit widerrufen"],
    ["analytics ", "we use google analytics"],
    [" consenso"],
    ["accept or "],
    ["utilisons"],
    ["cookies"],
    ["consentement"],
    ["partenaires"],
    ["publicit\xE9s"],
    ["utilise"],
    ["utilisent"],
    ["stocker"],
    ["consentement"],
    ["consentement"],
    ["utilisation"],
    ["technologies"],
    ["utilisation"],
    ["continuer sans accepter"],
    [" tous les cookies"],
    ["je refuse"],
    ["refuser et continuer"],
    ["en cliquant sur"],
    ["vous consentez"],
    ["accepter ou refuser "],
    ["les cookies nous "],
    ["optionnels", "publicitaires", "non essentiels"],
    ["seulement n\xE9cessaires"],
    ["je d\xE9sactive les finalit\xE9s non essentielles"],
    ["cookies essentiels uniquement"],
    ["n\xE9cessaires uniquement"],
    ["stocker", "stockons"],
    ["nos partenaires demandent votre consentement"],
    ["consentez \xE0 l", " cookies"],
    ["les cookies nous permettent", "nos partenaires et nous"],
    ["accepter "],
    ["on utilise", "nous utilisons"],
    ["naviguant", "poursuivant"],
    [],
    ["confidentialit\xE9"],
    ["verwenden"],
    ["partner"],
    ["zugriff auf informationen auf"],
    ["werbeleistung"],
    ["cookies"],
    ["verwendung"],
    ["cookies"],
    ["verwendet"],
    ["auswahl"],
    ["technologien"],
    ["verwenden"],
    ["zus\xE4tzliche", "essenzielle", "funktionale", "notwendige", "erforderliche"],
    ["einwilligung", "zustimmung", "cookies"],
    ["ablehnen"],
    ["nur funktionale cookies akzeptieren"],
    ["optionale ablehnen"],
    ["zustimmung verweigern"],
    ["speichern und/oder "],
    ["unsere partner bitten um ihre zustimmung"],
    [" cookies"],
    ["cookies "],
    [" zum einsatz", "einsatz von cookies"],
    ["einverstanden", "einverstanden"],
    ["akzeptieren", "zulassen", "erlauben", "zustimmen"],
    ["zustimmen und weiter", " ohne zu "],
    ["einwilligung", "zustimmung", "zustimmung", "einwilligung"],
    ["tracking-technologien"],
    ["cookies"],
    ["cookies"],
    ["cookies"],
    ["cookies"],
    ["datenschutz"],
    ["verwendung "],
    ["mit werbetracking weiterlesen", "weiter mit werbung", "con tracking e annunci"],
    ["cookies"],
    [],
    ["analyse", "statistik"],
    ["marketing", "tracking"],
    ["cookies oder "],
    ["stimmen sie", "gestatten sie", "willigen sie", "erkl\xE4ren sie sich"],
    ["ben\xF6tigen wir "],
    ["einwilligung", "zustimmung", "einwilligung", "zustimmung"],
    ["welche cookies sie "],
    ["cookie"],
    ["in form von cookies", "blockierung bestimmter arten von cookies"],
    ["notwendige", "essenzielle", "funktionale"],
    ["cookies"],
    ["zwecken und anbietern"],
    ["weiter ohne akzeptanz"],
    ["gebruik"],
    ["gebruiken"],
    ["vergelijkbare"],
    ["afwijzen", "weigeren", "verwerpen"],
    ["noodzakelijk"],
    ["cookies weigeren"],
    ["cookies"],
    ["doorgaan zonder "],
    ["optionele", "functionele", "functioneel", "noodzakelijke", "essenti\xEBle", "standaard"],
    [],
    ["onze partners vragen om "],
    ["wij en onze "],
    [" cookies"],
    ["accepteer alle cookies"],
    ["analytische", "functionele", "essenti\xEBle", "noodzakelijke"],
    [],
    ["ervaring", "gebruikerservaring"],
    ["gebruiken", "plaatsen"],
    ["gebruiken "],
    ["plaatsen van"],
    ["te blijven gebruiken"],
    ["te klikken"],
    ["alle cookies accepteren"],
    ["toestemming voor "],
    [" contin"],
    ["cookie"],
    [" uso de cookies"],
    ["usa cookies de google"],
    [" uso de cookies"],
    ["al utilizar nuestro sitio web"],
    ["almacenar la informaci\xF3n en un dispositivo y"],
    [" utiliza"],
    ["cookies propias y de"],
    ["son necesarias"],
    [" cookie"],
    ["navegando"],
    ["nosotros y nuestros"],
    ["recopilamos y almacenamos datos de usted y de su dispositivo"],
    ["utilizamos tecnolog"],
    ["almacenar", "almacenamos"],
    ["nuestros socios solicitan su consentimiento"],
    ["tanto nosotros como", "nosotros y"],
    ["continuar sin aceptar", "solo las necesarias"],
    ["uso de "],
    ["cookies "],
    ["las cookies nos "],
    ["instalamos", "empleamos"],
    ["cookies", "necesitamos usar cookies"],
    ["al hacer clic en"],
    [" todas las cookies"],
    ["estrictamente necesarias"],
    ["almacenar o acceder a "],
    ["as de seguimiento"],
    ["cookie"],
    ["u\u017Cywa\u0107", "u\u017Cywamy"],
    ["cele przetwarzania twoich danych przez zaufanych partner\xF3w iab"],
    ["dzi\u0119ki "],
    [" plik\xF3w cookie"],
    ["korzystamy z technologii, takich jak pliki cookie"],
    ["korzystamy"],
    ["niekt\xF3re pliki cookies"],
    ["pliki cookies i pokrewne im technologie umo\u017Cliwiaj\u0105 poprawne dzia\u0142anie strony i pomagaj\u0105 nam dostosowa\u0107 ofert\u0119 do twoich potrzeb"],
    ["przechowywanie informacji na urz\u0105dzeniu lub dost\u0119p do nich"],
    ["przechowywanie plik\xF3w cookie na swoim urz\u0105dzeniu"],
    ["przechowywa\u0107 i uzyskiwa\u0107 dost\u0119p do informacji na twoich urz\u0105dzeniach"],
    ["przetwarzamy"],
    [" u\u017Cywa "],
    ["ta strona korzysta z ciasteczek"],
    ["uzyskujemy dost\u0119p i przechowujemy informacje na urz\u0105dzeniu"],
    ["u\u017Cywa plik"],
    ["u\u017Cywamy plik\xF3w"],
    ["wykorzystuj\u0105 "],
    ["za pomoc\u0105 plik\xF3w cookies"],
    ["zgod\u0105 my i nasi partnerzy mo\u017Cemy wykorzystywa\u0107 precyzyjne dane geolokalizacyjne i identyfikacj\u0119"],
    ["cookies pr\xF2pies i de tercers"],
    ["utilitzem galetes"],
    ["\xFAnicament utilitza galetes pr\xF2pies amb finalitat t\xE8cnica"],
    ["este lloc web utilitza nom\xE9s cookies t\xE8cniques necess\xE0ries per al seu funcionament"],
    ["de personalitzaci\xF3 i an\xE0lisi"],
    ["utilitzem cookies i altres tecnologies"],
    ["cookies", "galetes"],
    ["cookie propio eta hirugarrenenak helburu teknikoarekin erabiltzen ditu"],
    ["cookie propioak eta hirugarrenen cookieak erabiltzen ditugu"],
    ["cookie propioak eta hirugarrenenak helburu teknikoarekin erabiltzen ditu"],
    ["ak erabiltzen ditu"],
    ["cookieak erabiltzen ditu"],
    [" bazkideek cookieak eta identifikadoreak erabiltzen ditugu"],
    ["norberaren eta hirugarrenen cookie"],
    ["web orri honek cookieak erabiltzen ditu"],
    ["webgune honek cookie propioak eta hirugarrenen cookie-fitxategiak erabiltzen ditu"],
    ["empregamos cookies propias"],
    ["este portal emprega cookies propias ou de terceiros con fins anal\xEDticos"],
    ["\u0438\u0441\u043F\u043E\u043B\u044C\u0437\u0443"],
    ["\u0438\u0441\u043F\u043E\u043B\u044C\u0437\u0443"],
    ["cookie", "\u043A\u0443\u043A\u0438"],
    ["\u0442\u0435\u0445\u043D\u043E\u043B\u043E\u0433\u0438"],
    ["cookie", "\u043A\u0443\u043A\u0438"],
    [],
    ["\u0431\u0440\u0430\u0443\u0437\u0435\u0440"],
    ["usiamo", "utilizziamo"],
    ["cookie"],
    ["noi e i nostri"],
    [" cookie"],
    [" cookie"],
    ["tecnici", "di profilazione", "di terze parti", "analitici", "statistici", "di marketing"],
    ["acconsent", "accett"],
    [" navigazione"],
    [" senza accettare"],
    ["rifiuta e abbonati"],
    ["consenso "],
    [" nostri "],
    ["terze parti selezionate"],
    ["archiviare", "archiviamo", "memorizzare", "memorizziamo"],
    [" dei cookie"],
    ["utilizz"],
    ["questo", "il nostro"],
    ["tecnologie "],
    ["cliccando su"],
    [" tutti i cookie"],
    ["noi e i nostri "],
    ["essenziali", "necessari"],
    ["anv\xE4nder"],
    ["cookies", "kakor"],
    ["anv\xE4nds ", "anv\xE4nds"],
    ["anv\xE4ndning", "anv\xE4ndningen", "anv\xE4ndandet"],
    ["godk\xE4nn", "accepter", "till\xE5t", "du till\xE5ter"],
    [" n\xF6dv\xE4ndiga"],
    ["lagra och/eller "],
    [" partner"],
    ["tekniker "],
    ["cookie", "\u30AF\u30C3\u30AD\u30FC"],
    ["\u95B2\u89A7\u5C65\u6B74", "\u7AEF\u672B\u60C5\u5831", "\u5229\u7528\u8005\u60C5\u5831"],
    [" cookies"],
    ["informasjonskapsler", "cookies"],
    ["vi benytter"],
    ["bruk av "]
  ];

  // lib/heuristic-prefilter.ts
  function literalIndex(all) {
    const distinct = [...new Set(all)].sort((a, b) => b.length - a.length);
    if (distinct.length === 0) {
      return { present: () => /* @__PURE__ */ new Set() };
    }
    const escape = (l) => l.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");
    const any = new RegExp(`(?=(${distinct.map(escape).join("|")}))`, "g");
    const implies = /* @__PURE__ */ new Map();
    const impliedBy = (m) => {
      let list = implies.get(m);
      if (!list) {
        list = distinct.filter((l) => m.startsWith(l));
        implies.set(m, list);
      }
      return list;
    };
    return {
      present(lower) {
        const found = /* @__PURE__ */ new Set();
        for (const m of lower.matchAll(any)) {
          if (!found.has(m[1])) {
            impliedBy(m[1]).forEach((l) => found.add(l));
          }
        }
        return found;
      }
    };
  }

  // lib/heuristic-detect.ts
  var TEXT_LIMIT = 1e5;
  var DEFAULT_WINDOW = 150;
  var builtinLiterals;
  function literalsOf(pattern) {
    if (!builtinLiterals) {
      const fresh = DETECT_PATTERN_LITERALS.length === DETECT_PATTERNS.length;
      builtinLiterals = new Map(fresh ? DETECT_PATTERNS.map((p, i) => [patternOf(p), DETECT_PATTERN_LITERALS[i]]) : []);
    }
    return builtinLiterals.get(pattern) ?? [];
  }
  var index;
  var literalsIn = (lower) => (index ??= literalIndex(DETECT_PATTERN_LITERALS.flat())).present(lower);
  var truncate = (text) => (text || "").slice(0, TEXT_LIMIT);
  var forInput = (scoped, input) => !scoped.input || scoped.input === input;
  var stateless = (re) => new RegExp(re.source, re.flags.replace(/[gy]/g, ""));
  function toPredicate(test) {
    if (typeof test === "function") {
      return test;
    }
    const re = stateless(test);
    return (text) => re.test(text);
  }
  var compiledPatterns = /* @__PURE__ */ new WeakMap();
  var compiledNegatives = /* @__PURE__ */ new WeakMap();
  function compilePatterns(detectPatterns) {
    let compiled = compiledPatterns.get(detectPatterns);
    if (!compiled) {
      compiled = detectPatterns.map((p) => {
        const pattern = patternOf(p);
        const input = p instanceof RegExp ? void 0 : p.input;
        const global = pattern.flags.includes("g") ? pattern : new RegExp(pattern.source, `${pattern.flags}g`);
        return { id: pattern.toString(), pattern, global, input, literals: literalsOf(pattern) };
      });
      compiledPatterns.set(detectPatterns, compiled);
    }
    return compiled;
  }
  function compileNegatives(negatives) {
    let compiled = compiledNegatives.get(negatives);
    if (!compiled) {
      compiled = negatives.map((n) => {
        const pattern = n.pattern ? stateless(n.pattern) : null;
        const unless = n.unless ? toPredicate(n.unless) : null;
        return {
          ...n,
          hits: (context) => (!pattern || pattern.test(context)) && !(unless && unless(context)),
          applies: n.appliesTo ? new Set(n.appliesTo.map(String)) : null
        };
      });
      compiledNegatives.set(negatives, compiled);
    }
    return compiled;
  }
  function negativeContext(text, start, end, negative) {
    if (negative.whole) {
      return text;
    }
    if (negative.lines === void 0) {
      const w = negative.window ?? DEFAULT_WINDOW;
      return text.slice(Math.max(0, start - w), end + w);
    }
    const lineEnd = (i) => {
      const next = text.indexOf("\n", i);
      return next === -1 ? text.length : next;
    };
    const [before, after] = Array.isArray(negative.lines) ? negative.lines : [negative.lines, negative.lines];
    let from = start > 0 ? text.lastIndexOf("\n", start - 1) + 1 : 0;
    let to = lineEnd(end);
    for (let i = 0; i < before; i++) {
      from = from >= 2 ? text.lastIndexOf("\n", from - 2) + 1 : 0;
    }
    for (let i = 0; i < after; i++) {
      to = to < text.length ? lineEnd(to + 1) : to;
    }
    return text.slice(from, to);
  }
  function checkHeuristicPatterns(allText, input, options = {}) {
    return matchHeuristicPatterns(allText, input, options, false);
  }
  function hasHeuristicPatterns(allText, input, options = {}) {
    return matchHeuristicPatterns(allText, input, options, true).patterns.length > 0;
  }
  function matchHeuristicPatterns(allText, input, { patterns: detectPatterns = DETECT_PATTERNS, negatives = DETECT_CONTEXT_NEGATIVES }, firstOnly) {
    if (input !== void 0 && input !== "popup" && input !== "page") {
      const got = Array.isArray(input) ? "a pattern list (pass it as the `patterns` option)" : JSON.stringify(input);
      throw new TypeError(`heuristic input must be 'popup', 'page' or undefined, got ${got}`);
    }
    allText = truncate(allText);
    const patterns = [];
    const snippets2 = [];
    const suppressed = [];
    const applicable = compileNegatives(negatives).filter((n) => forInput(n, input));
    let present;
    const wholeHits = /* @__PURE__ */ new Map();
    const hitsAt = (n, start, end) => {
      if (!n.whole) {
        return n.hits(negativeContext(allText, start, end, n));
      }
      let hit = wholeHits.get(n);
      if (hit === void 0) {
        hit = n.hits(allText);
        wholeHits.set(n, hit);
      }
      return hit;
    };
    for (const d of compilePatterns(detectPatterns)) {
      if (!forInput(d, input)) {
        continue;
      }
      if (d.literals.length) {
        const inText = present ??= literalsIn(allText.toLowerCase());
        if (!d.literals.some((l) => inText.has(l))) {
          continue;
        }
      }
      const own = applicable.filter((n) => !n.applies || n.applies.has(d.id));
      const matched = [];
      const dropped = /* @__PURE__ */ new Set();
      let found = false;
      let kept = own.length === 0;
      for (const m of allText.matchAll(d.global)) {
        found = true;
        if (!kept) {
          const start = m.index ?? 0;
          const hit = own.find((n) => hitsAt(n, start, start + m[0].length));
          if (hit) {
            dropped.add(hit.name);
          } else {
            kept = true;
          }
        }
        if (kept && firstOnly) {
          break;
        }
        matched.push(m[0].substring(0, 200));
      }
      if (!found) {
        continue;
      }
      if (!kept) {
        dropped.forEach((negative) => suppressed.push({ pattern: d.id, negative }));
        continue;
      }
      patterns.push(d.id);
      if (firstOnly) {
        break;
      }
      snippets2.push(...matched);
    }
    return { patterns, snippets: snippets2, suppressed };
  }

  // lib/heuristic-classify.ts
  function classifyButtons(buttons) {
    for (const button of buttons) {
      button.regexClassification = classifyButtonTextRegex(button.text);
    }
  }
  function classifyPopup(buttons) {
    const { reject, settings, accept, acknowledge } = buttons.reduce(
      (acc, button) => {
        if (button.regexClassification && button.regexClassification !== "other") {
          acc[button.regexClassification]++;
        }
        return acc;
      },
      { reject: 0, settings: 0, accept: 0, acknowledge: 0 }
    );
    if (reject > 0) {
      return "reject";
    }
    if (settings > 0) {
      return "none";
    }
    if (acknowledge > 0) {
      return "tier1";
    }
    if (accept > 0) {
      return accept === 1 ? "tier2" : "none";
    }
    return "none";
  }
  function testButtonMatches(buttonText, matchPatterns, neverMatchPatterns) {
    if (!buttonText) {
      return false;
    }
    const cleanedButtonText = cleanButtonText(buttonText);
    return !neverMatchPatterns.some((p) => p instanceof RegExp && p.test(cleanedButtonText) || p === cleanedButtonText) && matchPatterns.some((p) => p instanceof RegExp && p.test(cleanedButtonText) || p === cleanedButtonText);
  }
  function cleanButtonText(buttonText) {
    let result = buttonText.toLowerCase();
    result = result.replace(/[“”"'/#&[\]→✕×⟩❯><✗×‘’›«»]+/g, "");
    result = result.replace(
      /[\u{1F600}-\u{1F64F}\u{1F300}-\u{1F5FF}\u{1F680}-\u{1F6FF}\u{1F1E0}-\u{1F1FF}\u2600-\u26FF\u2700-\u27BF\u{1F900}-\u{1F9FF}\u{1FA70}-\u{1FAFF}]/gu,
      ""
    );
    result = result.replace(/\n+/g, " ");
    result = result.replace(/\s+/g, " ");
    result = result.trim();
    return result;
  }
  function classifyButtonTextRegex(buttonText) {
    if (testButtonMatches(buttonText, REJECT_PATTERNS, BUTTON_NEVER_MATCH_PATTERNS)) {
      return "reject";
    }
    if (testButtonMatches(buttonText, SETTINGS_PATTERNS, BUTTON_NEVER_MATCH_PATTERNS)) {
      return "settings";
    }
    if (testButtonMatches(buttonText, ACCEPT_PATTERNS, BUTTON_NEVER_MATCH_PATTERNS)) {
      return "accept";
    }
    if (testButtonMatches(buttonText, ACKNOWLEDGE_PATTERNS, BUTTON_NEVER_MATCH_PATTERNS)) {
      return "acknowledge";
    }
    return "other";
  }

  // lib/heuristics.ts
  var BUTTON_LIKE_ELEMENT_SELECTOR = 'button, input[type="button"], input[type="submit"], a, [role="button"], [class*="button"]';
  var POPUP_SEARCH_MAX_TIME = 100;
  var FRAGMENT_NEGATIVES = DETECT_CONTEXT_NEGATIVES.filter((n) => !n.whole);
  var hasFragmentPatterns = (text) => hasHeuristicPatterns(text, "popup", { negatives: FRAGMENT_NEGATIVES });
  var ACCEPTED_LEVELS = {
    off: [],
    reject: ["reject"],
    tier1: ["reject", "tier1"],
    tier2: ["reject", "tier1", "tier2"]
  };
  function getActionablePopups(mode = "reject", timeout = POPUP_SEARCH_MAX_TIME, hitTest = false) {
    const acceptedLevels = ACCEPTED_LEVELS[mode];
    const isActionable = (popup) => acceptedLevels.includes(popup.regexClassification) && isConsentPopup(popup);
    const popups = searchPopups(timeout, hitTest, DEFAULT_HIT_TEST_GRID, true).filter(isActionable);
    return popups.sort((a, b) => classificationRank(a) - classificationRank(b));
  }
  function searchPopups(timeout, hitTest, hitTestGrid, stopOnReject = false) {
    if (!isTopFrame()) {
      return getFramePopups();
    }
    const deadline = performance.now() + timeout;
    const hitTestPopups = hitTest ? findPopupsByHitTest(deadline, hitTestGrid) : [];
    if (stopOnReject && hitTestPopups.some((p) => p.regexClassification === "reject" && isConsentPopup(p))) {
      return hitTestPopups;
    }
    const walkPopups = collectPositionedPopups(deadline);
    return [...walkPopups, ...mergeWalkOverlaps(hitTestPopups, walkPopups)];
  }
  var consentPopups = /* @__PURE__ */ new WeakMap();
  function classificationRank(popup) {
    return ACCEPTED_LEVELS.tier2.indexOf(popup.regexClassification);
  }
  function isConsentPopup(popup) {
    if (!ACCEPTED_LEVELS.tier2.includes(popup.regexClassification)) {
      return false;
    }
    let result = consentPopups.get(popup);
    if (result === void 0) {
      const popupText = popup.text.trim();
      result = !!popupText && hasHeuristicPatterns(popupText, "popup");
      consentPopups.set(popup, result);
    }
    return result;
  }
  function mergeWalkOverlaps(hitTestPopups, walkPopups) {
    return hitTestPopups.filter((h) => {
      const hitIsConsent = isConsentPopup(h);
      const absorbs = (p) => !hitIsConsent || isConsentPopup(p) && classificationRank(p) <= classificationRank(h);
      const walkPopup = walkPopups.find((p) => composedOverlaps(p.element, h.element) && absorbs(p));
      if (walkPopup) {
        (walkPopup.hitPoints ??= []).push(...h.hitPoints ?? []);
      }
      return !walkPopup;
    });
  }
  function getFramePopups() {
    if (window.parent && window.parent !== window.top) {
      return [];
    }
    const frameElement = getFramePopupElement(document);
    const popup = frameElement && toPopupData(frameElement);
    if (!popup) {
      return [];
    }
    popup.source = "frame";
    return [popup];
  }
  function collectPositionedPopups(deadline) {
    const { elements, shadowHosts } = getPopupLikeElements(deadline);
    const potentialPopups = [];
    const expanded = [];
    for (const el of elements) {
      const popup = toPopupData(el, shadowHosts);
      if (!popup) {
        continue;
      }
      const root = performance.now() > deadline ? popup : expandPopupFragment(popup, shadowHosts);
      if (potentialPopups.some((p) => p.element === root.element)) {
        continue;
      }
      if (root !== popup) {
        expanded.push(root.element);
      }
      root.source = "walk";
      potentialPopups.push(root);
    }
    return potentialPopups.filter((p) => !expanded.some((root) => root !== p.element && composedContains(root, p.element)));
  }
  var DEFAULT_HIT_TEST_GRID = {
    columns: [0.1, 0.5, 0.9],
    // banners sit along the top and bottom edges and modals in the middle; rows 150px in added almost nothing in crawls
    edgeRows: [30, 90]
  };
  function findPopupsByHitTest(deadline, grid) {
    const climbed = /* @__PURE__ */ new Map();
    const found = [];
    const toPopup = (el) => el.querySelector(TEXT_ENTRY_SELECTOR) ? null : toPopupRoot(el);
    for (const [x, y] of getHitTestPoints(grid)) {
      if (performance.now() > deadline) {
        return found;
      }
      const path = [];
      let reached = null;
      const shouldStop = (el) => {
        if (climbed.has(el)) {
          reached = climbed.get(el) ?? null;
          return true;
        }
        const overlapped = found.find((p) => composedOverlaps(p.element, el));
        if (overlapped) {
          reached = composedContains(overlapped.element, el) ? overlapped : null;
          return true;
        }
        path.push(el);
        return false;
      };
      const hit = closestHTMLElement(deepElementFromPoint(x, y));
      const popup = climbToPopup(hit, MAX_HIT_TEST_CLIMB_DEPTH, toPopup, shouldStop);
      if (popup) {
        popup.source = "hitTest";
        found.push(popup);
        reached = popup;
      }
      for (const el of path) {
        climbed.set(el, reached);
      }
      if (reached) {
        (reached.hitPoints ??= []).push([x, y]);
      }
    }
    return found;
  }
  function getHitTestPoints(grid) {
    const { innerWidth: width, innerHeight: height } = window;
    const rows = [...grid.edgeRows, height / 2, ...grid.edgeRows.map((y) => height - y)].map((y) => Math.round(y));
    const ys = [...new Set(rows)].filter((y) => y >= 0 && y < height);
    return ys.flatMap((y) => grid.columns.map((column) => [Math.round(column * width), y]));
  }
  var TEXT_ENTRY_SELECTOR = 'textarea, input:not([type]), input[type="text"], input[type="email"], input[type="password"], input[type="search"], input[type="tel"], input[type="number"], input[type="url"]';
  function deepElementFromPoint(x, y) {
    let el = document.elementFromPoint(x, y);
    while (el?.shadowRoot) {
      const inner = el.shadowRoot.elementFromPoint(x, y);
      if (!inner || inner === el) {
        break;
      }
      el = inner;
    }
    return el;
  }
  function closestHTMLElement(el) {
    while (el && !(el instanceof HTMLElement)) {
      el = el.parentElement;
    }
    return el;
  }
  function toPopupData(el, shadowHosts, text = getElementText(el)) {
    if (!text) {
      return null;
    }
    const buttons = getButtonData(el, shadowHosts);
    classifyButtons(buttons);
    return { text, element: el, buttons, regexClassification: classifyPopup(buttons) };
  }
  function hasPopupButtons(text, buttons) {
    return !isOnlyButtonText(text, buttons) && buttons.some((b) => b.regexClassification !== "other");
  }
  function isOnlyButtonText(text, buttons) {
    let rest = text;
    for (const button of buttons) {
      rest = rest.replace(button.text, "");
    }
    return rest.replace(/\s+/g, "").length < 10;
  }
  function toPopupRoot(el, shadowHosts) {
    const text = getElementText(el);
    if (!text || !hasFragmentPatterns(text)) {
      return null;
    }
    const popup = toPopupData(el, shadowHosts, text);
    return popup && hasPopupButtons(text, popup.buttons) ? popup : null;
  }
  var MAX_FRAGMENT_EXPANSION_DEPTH = 6;
  var MAX_HIT_TEST_CLIMB_DEPTH = 8;
  function climbToPopup(el, maxDepth, toPopup, shouldStop) {
    for (let depth = 0; el && depth < maxDepth; depth++, el = getComposedParent(el)) {
      if (shouldStop?.(el) || el === document.body || el === document.documentElement || el.getBoundingClientRect().height > window.innerHeight) {
        return null;
      }
      const popup = toPopup(el);
      if (popup) {
        return popup;
      }
    }
    return null;
  }
  function expandPopupFragment(popup, shadowHosts) {
    if (hasFragmentPatterns(popup.text) && hasPopupButtons(popup.text, popup.buttons)) {
      return popup;
    }
    const parent = getComposedParent(popup.element);
    return climbToPopup(parent, MAX_FRAGMENT_EXPANSION_DEPTH, (el) => toPopupRoot(el, shadowHosts)) ?? popup;
  }
  function getComposedParent(el) {
    if (el.parentElement) {
      return el.parentElement;
    }
    const parent = el.parentNode;
    return parent instanceof ShadowRoot && parent.host instanceof HTMLElement ? parent.host : null;
  }
  function getFramePopupElement(doc) {
    const el = doc.body || doc.documentElement;
    const win = doc.defaultView;
    if (!el || !win || win.innerWidth === 0 || win.innerHeight === 0 || !isStyleVisible(el)) {
      return null;
    }
    return getElementText(el).trim() ? el : null;
  }
  function isStyleVisible(el) {
    return typeof el.checkVisibility !== "function" || el.checkVisibility({ checkVisibilityCSS: true });
  }
  function getElementText(el) {
    const text = el.innerText || "";
    if (!el.shadowRoot) {
      return text;
    }
    const shadowTexts = Array.from(el.shadowRoot.children, (child) => isRenderedTextElement(child) ? getElementText(child) : "");
    return [text, ...shadowTexts].filter((t) => t.trim()).join("\n");
  }
  function isRenderedTextElement(el) {
    return el instanceof HTMLElement && !["STYLE", "SCRIPT", "TEMPLATE", "NOSCRIPT", "LINK", "SLOT"].includes(el.tagName) && window.getComputedStyle(el).display !== "none";
  }
  function composedOverlaps(a, b) {
    return composedContains(a, b) || composedContains(b, a);
  }
  function composedContains(container, node) {
    if (container.contains(node)) {
      return true;
    }
    let root = node.getRootNode();
    while (root instanceof ShadowRoot) {
      if (container.contains(root.host)) {
        return true;
      }
      root = root.host.getRootNode();
    }
    return false;
  }
  function isPopupCandidateVisible(el) {
    return isElementVisible(el) && isStyleVisible(el) && isOnScreen(el);
  }
  function isOnScreen(el) {
    const rect = el.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) {
      return window.getComputedStyle(el).overflow === "visible" && Array.from(el.children).some((child) => isOnScreen(child));
    }
    return rect.bottom > 0 && rect.right > 0 && rect.top < window.innerHeight && rect.left < window.innerWidth;
  }
  function isDialogLikeElement(node) {
    if (node.tagName === "DIALOG" && node.hasAttribute("open")) {
      return true;
    }
    if (node.getAttribute("role") === "dialog" || node.getAttribute("aria-modal") === "true") {
      return true;
    }
    return false;
  }
  function getPopupLikeElements(deadline) {
    const found = [];
    const shadowHosts = [];
    const roots = [document.documentElement];
    for (let i = 0; i < roots.length; i++) {
      if (i > 0 && performance.now() > deadline) {
        break;
      }
      const walker = document.createTreeWalker(
        roots[i],
        NodeFilter.SHOW_ELEMENT,
        // visit only element nodes
        {
          acceptNode(node) {
            if (node.shadowRoot) {
              shadowHosts.push(node);
              roots.push(node.shadowRoot);
            }
            if (node.tagName === "BODY") {
              return NodeFilter.FILTER_SKIP;
            }
            const style = window.getComputedStyle(node);
            if (style.display === "none") {
              return NodeFilter.FILTER_REJECT;
            }
            const cssPosition = style.position;
            if ((cssPosition === "fixed" || cssPosition === "sticky" || isDialogLikeElement(node)) && isPopupCandidateVisible(node)) {
              return NodeFilter.FILTER_ACCEPT;
            }
            if (performance.now() > deadline) {
              return NodeFilter.FILTER_REJECT;
            }
            return NodeFilter.FILTER_SKIP;
          }
        }
      );
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        found.push(node);
      }
    }
    return { elements: excludeContainers(found.filter((element) => getElementText(element).trim())), shadowHosts };
  }
  function getButtonData(el, shadowHosts) {
    const buttons = [];
    for (const b of excludeContainers(getButtonLikeElements(el, shadowHosts))) {
      if (!isElementVisible(b) || isDisabled(b)) {
        continue;
      }
      const text = getElementText(b).trim() || // <input> values do not appear in innerText
      (b instanceof HTMLInputElement && ["submit", "button"].includes(b.type) ? b.value?.trim() : "");
      if (text) {
        buttons.push({ text, element: b });
      }
    }
    return buttons;
  }
  function getButtonLikeElements(el, shadowHosts = findShadowHosts(el)) {
    const buttons = Array.from(el.querySelectorAll(BUTTON_LIKE_ELEMENT_SELECTOR));
    for (const host of shadowHosts) {
      if (composedContains(el, host)) {
        buttons.push(...host.shadowRoot.querySelectorAll(BUTTON_LIKE_ELEMENT_SELECTOR));
      }
    }
    return buttons;
  }
  function findShadowHosts(root) {
    const hosts = [];
    const candidates = root instanceof HTMLElement ? [root, ...root.querySelectorAll("*")] : root.querySelectorAll("*");
    for (const el of candidates) {
      if (el.shadowRoot) {
        hosts.push(el, ...findShadowHosts(el.shadowRoot));
      }
    }
    return hosts;
  }
  function isDisabled(el) {
    return "disabled" in el && Boolean(el.disabled) || el.hasAttribute("disabled");
  }
  function excludeContainers(elements) {
    const candidates = new Set(elements);
    const containers = /* @__PURE__ */ new Set();
    for (const el of elements) {
      for (let node = el.parentNode; node; node = node.parentNode || (node instanceof ShadowRoot ? node.host : null)) {
        if (candidates.has(node)) {
          containers.add(node);
        }
      }
    }
    return elements.filter((el) => !containers.has(el)).reverse();
  }

  // lib/cmps/base.ts
  var defaultRunContext = {
    main: true,
    frame: false,
    urlPattern: ""
  };
  var AutoConsentCMPBase = class {
    constructor(autoconsentInstance) {
      this.name = "BASERULE";
      this.runContext = defaultRunContext;
      this.autoconsent = autoconsentInstance;
    }
    get hasSelfTest() {
      throw new Error("Not Implemented");
    }
    get isIntermediate() {
      throw new Error("Not Implemented");
    }
    get isCosmetic() {
      throw new Error("Not Implemented");
    }
    mainWorldEval(snippetId) {
      const snippet = snippets[snippetId];
      if (!snippet) {
        this.autoconsent.config.logs.errors && console.warn("Snippet not found", snippetId);
        return Promise.resolve(false);
      }
      const logsConfig = this.autoconsent.config.logs;
      if (this.autoconsent.config.isMainWorld) {
        logsConfig.evals && console.log("inline eval:", snippetId, snippet);
        let result = false;
        try {
          result = !!snippet.call(globalThis);
        } catch (e) {
          logsConfig.evals && console.error("error evaluating rule", snippetId, e);
        }
        return Promise.resolve(result);
      }
      const snippetSrc = getFunctionBody(snippet);
      logsConfig.evals && console.log("async eval:", snippetId, snippetSrc);
      return requestEval(snippetSrc, snippetId).catch((e) => {
        logsConfig.evals && console.error("error evaluating rule", snippetId, e);
        return false;
      });
    }
    checkRunContext() {
      if (!this.checkFrameContext(isTopFrame())) {
        return false;
      }
      if (this.runContext.urlPattern && !this.hasMatchingUrlPattern()) {
        return false;
      }
      return true;
    }
    checkFrameContext(isTop) {
      const runCtx = {
        ...defaultRunContext,
        ...this.runContext
      };
      if (isTop && !runCtx.main) {
        return false;
      }
      if (!isTop && !runCtx.frame) {
        return false;
      }
      return true;
    }
    hasMatchingUrlPattern() {
      return Boolean(this.runContext?.urlPattern && window.location.href.match(this.runContext.urlPattern));
    }
    detectCmp() {
      throw new Error("Not Implemented");
    }
    async detectPopup() {
      return false;
    }
    optOut() {
      throw new Error("Not Implemented");
    }
    optIn() {
      throw new Error("Not Implemented");
    }
    openCmp() {
      throw new Error("Not Implemented");
    }
    async test() {
      return Promise.resolve(true);
    }
    async highlightElements(elements, all = false, delayTimeout = 2e3) {
      if (elements.length === 0) {
        return;
      }
      if (!all) {
        elements = [elements[0]];
      }
      this.autoconsent.sendContentMessage({
        type: "visualDelay",
        timeout: delayTimeout
      });
      for (const el of elements) {
        this.autoconsent.config.logs.rulesteps && console.log("highlighting", el);
        highlightNode(el);
      }
      await this.wait(delayTimeout);
      for (const el of elements) {
        unhighlightNode(el);
      }
    }
    // Implementing DomActionsProvider below:
    async clickElement(element) {
      if (this.autoconsent.config.visualTest) {
        await this.highlightElements([element]);
      }
      this.autoconsent.updateState({ clicks: this.autoconsent.state.clicks + 1 });
      return this.autoconsent.domActions.clickElement(element);
    }
    async click(selector, all = false) {
      if (this.autoconsent.config.visualTest) {
        await this.highlightElements(this.elementSelector(selector), all);
      }
      this.autoconsent.updateState({ clicks: this.autoconsent.state.clicks + 1 });
      return this.autoconsent.domActions.click(selector, all);
    }
    elementExists(selector) {
      return this.autoconsent.domActions.elementExists(selector);
    }
    elementVisible(selector, check) {
      return this.autoconsent.domActions.elementVisible(selector, check);
    }
    waitForElement(selector, timeout) {
      return this.autoconsent.domActions.waitForElement(selector, timeout);
    }
    waitForVisible(selector, timeout, check) {
      return this.autoconsent.domActions.waitForVisible(selector, timeout, check);
    }
    async waitForThenClick(selector, timeout, all, retries, retryInterval) {
      if (this.autoconsent.config.visualTest) {
        await this.highlightElements(this.elementSelector(selector), all);
      }
      this.autoconsent.updateState({ clicks: this.autoconsent.state.clicks + 1 });
      return this.autoconsent.domActions.waitForThenClick(selector, timeout, all, retries, retryInterval);
    }
    wait(ms) {
      return this.autoconsent.domActions.wait(ms);
    }
    hide(selector, method) {
      return this.autoconsent.domActions.hide(selector, method);
    }
    stylesheet(cssRule, stylesheetId) {
      return this.autoconsent.domActions.stylesheet(cssRule, stylesheetId);
    }
    removeClass(selector, className) {
      return this.autoconsent.domActions.removeClass(selector, className);
    }
    setStyle(selector, css) {
      return this.autoconsent.domActions.setStyle(selector, css);
    }
    addStyle(selector, css) {
      return this.autoconsent.domActions.addStyle(selector, css);
    }
    cookieContains(substring) {
      return this.autoconsent.domActions.cookieContains(substring);
    }
    prehide(selector) {
      return this.autoconsent.domActions.prehide(selector);
    }
    undoPrehide() {
      return this.autoconsent.domActions.undoPrehide();
    }
    querySingleReplySelector(selector, parent) {
      return this.autoconsent.domActions.querySingleReplySelector(selector, parent);
    }
    querySelectorChain(selectors) {
      return this.autoconsent.domActions.querySelectorChain(selectors);
    }
    elementSelector(selector) {
      return this.autoconsent.domActions.elementSelector(selector);
    }
    waitForMutation(selector) {
      return this.autoconsent.domActions.waitForMutation(selector);
    }
  };
  var AutoConsentCMP = class extends AutoConsentCMPBase {
    constructor(rule, autoconsentInstance) {
      super(autoconsentInstance);
      this.rule = rule;
      this.name = rule.name;
      this.runContext = rule.runContext || defaultRunContext;
    }
    get hasSelfTest() {
      return !!this.rule.test && this.rule.test.length > 0;
    }
    get isIntermediate() {
      return !!this.rule.intermediate;
    }
    get isCosmetic() {
      return !!this.rule.cosmetic;
    }
    get prehideSelectors() {
      return this.rule.prehideSelectors || [];
    }
    async detectCmp() {
      if (this.rule.detectCmp) {
        return this._runRulesSequentially(this.rule.detectCmp, this.autoconsent.config.logs.detectionsteps);
      }
      return false;
    }
    async detectPopup() {
      if (this.rule.detectPopup) {
        return this._runRulesSequentially(this.rule.detectPopup, this.autoconsent.config.logs.detectionsteps);
      }
      return false;
    }
    async optOut() {
      const logsConfig = this.autoconsent.config.logs;
      if (this.rule.optOut) {
        logsConfig.lifecycle && console.log("Initiated optOut()", this.rule.optOut);
        return this._runRulesSequentially(this.rule.optOut, this.autoconsent.config.logs.rulesteps);
      }
      return false;
    }
    async optIn() {
      const logsConfig = this.autoconsent.config.logs;
      if (this.rule.optIn) {
        logsConfig.lifecycle && console.log("Initiated optIn()", this.rule.optIn);
        return this._runRulesSequentially(this.rule.optIn, this.autoconsent.config.logs.rulesteps);
      }
      return false;
    }
    async openCmp() {
      if (this.rule.openCmp) {
        return this._runRulesSequentially(this.rule.openCmp, this.autoconsent.config.logs.rulesteps);
      }
      return false;
    }
    async test() {
      if (this.hasSelfTest && this.rule.test) {
        return this._runRulesSequentially(this.rule.test, this.autoconsent.config.logs.rulesteps);
      }
      return super.test();
    }
    async evaluateRuleStep(rule) {
      const results = [];
      const logsConfig = this.autoconsent.config.logs;
      if (rule.exists) {
        results.push(this.elementExists(rule.exists));
      }
      if (rule.visible) {
        results.push(this.elementVisible(rule.visible, rule.check));
      }
      if (rule.eval) {
        const res = this.mainWorldEval(rule.eval);
        results.push(res);
      }
      if (rule.waitFor) {
        results.push(this.waitForElement(rule.waitFor, rule.timeout));
      }
      if (rule.waitForVisible) {
        results.push(this.waitForVisible(rule.waitForVisible, rule.timeout, rule.check));
      }
      if (rule.click) {
        results.push(this.click(rule.click, rule.all));
      }
      if (rule.waitForThenClick) {
        results.push(this.waitForThenClick(rule.waitForThenClick, rule.timeout, rule.all, rule.retry, rule.retryInterval));
      }
      if (rule.wait) {
        results.push(this.wait(rule.wait));
      }
      if (rule.hide) {
        results.push(this.hide(rule.hide, rule.method));
      }
      if (rule.stylesheet !== void 0) {
        results.push(this.stylesheet(rule.stylesheet, rule.stylesheetId));
      }
      if (rule.removeClass !== void 0) {
        results.push(rule.selector ? this.removeClass(rule.selector, rule.removeClass) : false);
      }
      if (rule.setStyle !== void 0) {
        results.push(rule.selector ? this.setStyle(rule.selector, rule.setStyle) : false);
      }
      if (rule.addStyle !== void 0) {
        results.push(rule.selector ? this.addStyle(rule.selector, rule.addStyle) : false);
      }
      if (rule.cookieContains) {
        results.push(this.cookieContains(rule.cookieContains));
      }
      if (rule.if) {
        if (!rule.if.exists && !rule.if.visible) {
          console.error("invalid conditional rule", rule.if);
          return false;
        }
        if (!rule.then) {
          console.error('invalid conditional rule, missing "then" step', rule.if);
          return false;
        }
        const condition = await this.evaluateRuleStep(rule.if);
        logsConfig.rulesteps && console.log("Condition is", condition);
        if (condition) {
          results.push(this._runRulesSequentially(rule.then, logsConfig.rulesteps));
        } else if (rule.else) {
          results.push(this._runRulesSequentially(rule.else, logsConfig.rulesteps));
        } else {
          results.push(true);
        }
      }
      if (rule.any) {
        let resultOfAny = false;
        for (const step of rule.any) {
          if (await this.evaluateRuleStep(step)) {
            resultOfAny = true;
            break;
          }
        }
        results.push(resultOfAny);
      }
      if (results.length === 0) {
        logsConfig.errors && console.warn("Unrecognized rule", rule);
        return false;
      }
      const all = await Promise.all(results);
      const result = all.reduce((a, b) => a && b, true);
      if (rule.negated) {
        return !result;
      }
      return result;
    }
    async _runRulesParallel(rules) {
      const results = rules.map((rule) => this.evaluateRuleStep(rule));
      const detections = await Promise.all(results);
      return detections.every((r) => !!r);
    }
    async _runRulesSequentially(rules, logSteps = true) {
      for (const rule of rules) {
        logSteps && console.log("Running rule...", rule);
        const result = await this.evaluateRuleStep(rule);
        logSteps && console.log("...rule result", result);
        if (!result && !rule.optional) {
          return false;
        }
      }
      return true;
    }
  };
  var AutoConsentHeuristicCMP = class extends AutoConsentCMPBase {
    constructor(autoconsentInstance, mode = "reject") {
      super(autoconsentInstance);
      this.popups = [];
      this.name = "HEURISTIC";
      this.runContext = {
        main: true,
        frame: false
        // do not run in iframes for security reasons
      };
      this.mode = mode;
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      await new Promise((resolve) => setTimeout(resolve, 0));
      this.autoconsent.config.performanceLoggingEnabled && performance.mark("heuristicDetectorStart");
      this.popups = getActionablePopups(
        this.mode,
        this.autoconsent.config.heuristicPopupSearchTimeout,
        this.autoconsent.config.enableHeuristicHitTest
      );
      this.autoconsent.config.performanceLoggingEnabled && performance.mark("heuristicDetectorEnd");
      this.autoconsent.config.performanceLoggingEnabled && performance.measure("heuristicDetector", "heuristicDetectorStart", "heuristicDetectorEnd");
      if (this.popups.length > 0) {
        this.name = `HEURISTIC-${this.popups[0].regexClassification?.toUpperCase()}`;
        return Promise.resolve(true);
      }
      return Promise.resolve(false);
    }
    async detectPopup() {
      if (this.popups.length > 0) {
        if (this.popups.length > 1) {
          this.autoconsent.config.logs.errors && console.warn("Heuristic found multiple popups");
        }
        return true;
      }
      return false;
    }
    getTargetButton() {
      const popup = this.popups[0];
      const level = popup.regexClassification;
      const buttons = popup.buttons;
      const targetButtonType = level === "reject" ? "reject" : level === "tier1" ? "acknowledge" : "accept";
      return buttons.find((button) => button.regexClassification === targetButtonType);
    }
    optOut() {
      const button = this.getTargetButton();
      if (button) {
        return this.clickElement(button.element);
      }
      return Promise.resolve(false);
    }
    optIn() {
      throw new Error("Not Implemented");
    }
    openCmp() {
      throw new Error("Not Implemented");
    }
    async test() {
      const button = this.getTargetButton();
      if (button) {
        await this.wait(500);
        return !isElementVisible(button.element);
      }
      return false;
    }
  };

  // lib/cmps/trustarc-top.ts
  var cookieSettingsButton = "#truste-show-consent";
  var shortcutOptOut = "#truste-consent-required";
  var shortcutOptIn = "#truste-consent-button";
  var popupContent = "#truste-consent-content";
  var bannerOverlay = "#trustarc-banner-overlay";
  var bannerContainer = "#truste-consent-track";
  var prefCenterDeclineAll = [".truste_popframe.trustarc_newcm_container", 'button[data-button-key="decline_all"]'];
  var TrustArcTop = class extends AutoConsentCMPBase {
    constructor(autoconsentInstance) {
      super(autoconsentInstance);
      this.name = "TrustArc-top";
      this.prehideSelectors = [".trustarc-banner-container", `.truste_popframe,.truste_overlay,.truste_box_overlay,${bannerContainer}`];
      this.runContext = {
        main: true,
        frame: false
      };
      this._shortcutButton = null;
      this._optInDone = false;
      this._optOutDone = false;
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      if (this._optInDone || this._optOutDone) {
        return false;
      }
      return !this._shortcutButton;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      const result = this.elementExists(`${cookieSettingsButton},${bannerContainer}`);
      if (result) {
        this._shortcutButton = document.querySelector(shortcutOptOut);
      }
      return result;
    }
    async detectPopup() {
      return this.elementVisible(`${popupContent},${bannerOverlay},${bannerContainer}`, "any");
    }
    async optOut() {
      if (this.elementExists(shortcutOptOut)) {
        this.click(shortcutOptOut);
        return true;
      }
      hideElements(getStyleElement(), `.truste_popframe, .truste_overlay, .truste_box_overlay, ${bannerContainer}`);
      await this.click(cookieSettingsButton);
      if (await this.waitForElement(prefCenterDeclineAll, 3e3)) {
        this._optOutDone = true;
        await this.click(prefCenterDeclineAll);
      }
      setTimeout(() => {
        getStyleElement().remove();
      }, 1e4);
      return true;
    }
    async optIn() {
      this._optInDone = true;
      return await this.click(shortcutOptIn);
    }
    async openCmp() {
      return true;
    }
    async test() {
      await this.wait(500);
      return await this.mainWorldEval("EVAL_TRUSTARC_TOP");
    }
  };

  // lib/cmps/cookiebot.ts
  var Cookiebot = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Cybotcookiebot";
      this.prehideSelectors = [
        "#CybotCookiebotDialog,#CybotCookiebotDialogBodyUnderlay,#dtcookie-container,#cookiebanner,#cb-cookieoverlay,.modal--cookie-banner,#cookiebanner_outer,#CookieBanner"
      ];
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      return await this.mainWorldEval("EVAL_COOKIEBOT_1");
    }
    async detectPopup() {
      return this.mainWorldEval("EVAL_COOKIEBOT_2");
    }
    async optOut() {
      if (this.elementVisible("#CybotCookiebotDialogBodyLevelButtonLevelOptinDeclineAll")) {
        return await this.click("#CybotCookiebotDialogBodyLevelButtonLevelOptinDeclineAll");
      }
      await this.wait(500);
      let res = await this.mainWorldEval("EVAL_COOKIEBOT_3");
      await this.wait(1e3);
      res = res && await this.mainWorldEval("EVAL_COOKIEBOT_4");
      if (this.elementVisible("#CybotCookiebotDialogBodyButtonDecline")) {
        return await this.click("#CybotCookiebotDialogBodyButtonDecline");
      }
      return res;
    }
    async optIn() {
      if (this.elementExists("#dtcookie-container")) {
        return await this.click(".h-dtcookie-accept");
      }
      await this.click(".CybotCookiebotDialogBodyLevelButton:not(:checked):enabled", true);
      await this.click("#CybotCookiebotDialogBodyLevelButtonAccept");
      await this.click("#CybotCookiebotDialogBodyButtonAccept");
      return true;
    }
    async test() {
      await this.wait(500);
      return await this.mainWorldEval("EVAL_COOKIEBOT_5");
    }
  };

  // lib/cmps/sourcepoint-frame.ts
  var SourcePoint = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Sourcepoint-frame";
      this.prehideSelectors = ["div[id^='sp_message_container_'],.message-overlay", "#sp_privacy_manager_container"];
      this.ccpaNotice = false;
      this.ccpaPopup = false;
      this.runContext = {
        main: true,
        frame: true
      };
    }
    get hasSelfTest() {
      return false;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      const url = new URL(location.href);
      if (url.searchParams.has("message_id") && url.hostname === "ccpa-notice.sp-prod.net") {
        this.ccpaNotice = true;
        return true;
      }
      if (url.hostname === "ccpa-pm.sp-prod.net") {
        this.ccpaPopup = true;
        return true;
      }
      return (url.pathname === "/index.html" || url.pathname === "/privacy-manager/index.html" || url.pathname === "/ccpa_pm/index.html" || url.pathname === "/us_pm/index.html") && (url.searchParams.has("message_id") || url.searchParams.has("requestUUID") || url.searchParams.has("consentUUID"));
    }
    async detectPopup() {
      if (this.ccpaNotice) {
        return true;
      }
      if (this.ccpaPopup) {
        return await this.waitForElement(".priv-save-btn", 2e3);
      }
      await this.waitForElement(
        ".sp_choice_type_11,.sp_choice_type_12,.sp_choice_type_13,.sp_choice_type_ACCEPT_ALL,.sp_choice_type_SAVE_AND_EXIT",
        2e3
      );
      return !this.elementExists(".sp_choice_type_9");
    }
    async optIn() {
      await this.waitForElement(".sp_choice_type_11,.sp_choice_type_ACCEPT_ALL", 2e3);
      if (await this.click(".sp_choice_type_11")) {
        return true;
      }
      if (await this.click(".sp_choice_type_ACCEPT_ALL")) {
        return true;
      }
      return false;
    }
    isManagerOpen() {
      if (location.pathname === "/privacy-manager/index.html" || location.pathname === "/ccpa_pm/index.html") {
        return true;
      }
      if (location.pathname === "/us_pm/index.html" && !document.querySelector(".sp_choice_type_11,.sp_choice_type_ACCEPT_ALL")) {
        return true;
      }
      return false;
    }
    async optOut() {
      await this.wait(500);
      const logsConfig = this.autoconsent.config.logs;
      if (this.ccpaPopup) {
        const toggles = document.querySelectorAll(
          ".priv-purpose-container .sp-switch-arrow-block a.neutral.on .right"
        );
        for (const t of toggles) {
          t.click();
        }
        const switches = document.querySelectorAll(
          ".priv-purpose-container .sp-switch-arrow-block a.switch-bg.on"
        );
        for (const t of switches) {
          t.click();
        }
        return await this.click(".priv-save-btn");
      }
      if (this.elementVisible(".sp_choice_type_SE", "any")) {
        await this.click(
          [
            "xpath///div[contains(., 'Do not share my personal information') and contains(@class, 'switch-container')]",
            ".pm-switch[aria-checked=false] .slider"
          ],
          false
        );
        return await this.click(".sp_choice_type_SE");
      }
      if (!this.isManagerOpen()) {
        const manageSelector = '.sp_choice_type_12,[data-choice]:not([class*="sp_choice_type_"])';
        const actionable = await this.waitForVisible(`${manageSelector},.sp_choice_type_13`);
        if (!actionable) {
          return false;
        }
        if (this.elementVisible(".sp_choice_type_13", "any")) {
          return await this.click(".sp_choice_type_13");
        }
        await this.click(manageSelector);
        await waitFor(() => this.isManagerOpen(), 200, 100);
      }
      await this.waitForElement(".type-modal", 2e4);
      if (this.elementExists("[role=tablist]")) {
        await this.waitForElement("[role=tablist] [role=tab]", 1e4);
      }
      this.waitForThenClick(".ccpa-stack .pm-switch[aria-checked=true] .slider", 500, true);
      try {
        const rejectSelector1 = ".sp_choice_type_REJECT_ALL";
        const rejectSelector2 = ".reject-toggle";
        const path = await Promise.race([
          this.waitForElement(rejectSelector1, 2e3).then((success) => success ? 0 : -1),
          this.waitForElement(rejectSelector2, 2e3).then((success) => success ? 1 : -1),
          this.waitForElement(".pm-features", 2e3).then((success) => success ? 2 : -1)
        ]);
        if (path === 0) {
          await this.waitForVisible(rejectSelector1);
          return await this.click(rejectSelector1);
        } else if (path === 1) {
          await this.click(rejectSelector2);
        } else if (path === 2) {
          await this.waitForElement(".pm-features", 1e4);
          await this.click(".checked > span", true);
          await this.click(".chevron");
        }
      } catch (e) {
        logsConfig.errors && console.warn(e);
      }
      return await this.click(".sp_choice_type_SAVE_AND_EXIT");
    }
  };

  // lib/cmps/consentmanager.ts
  var ConsentManager = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "consentmanager.net";
      this.prehideSelectors = ["#cmpbox,#cmpbox2"];
      this.apiAvailable = false;
    }
    get hasSelfTest() {
      return this.apiAvailable;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      this.apiAvailable = await this.mainWorldEval("EVAL_CONSENTMANAGER_1");
      if (!this.apiAvailable) {
        return this.elementExists("#cmpbox");
      } else {
        return true;
      }
    }
    async detectPopup() {
      if (this.elementVisible("#cmpbox .cmpmore", "any")) {
        return true;
      } else if (this.apiAvailable) {
        await this.wait(500);
        return await this.mainWorldEval("EVAL_CONSENTMANAGER_2");
      }
      return false;
    }
    async optOut() {
      await this.wait(500);
      if (this.apiAvailable) {
        return await this.mainWorldEval("EVAL_CONSENTMANAGER_3");
      }
      if (await this.click(".cmpboxbtnno")) {
        return true;
      }
      if (this.elementExists(".cmpwelcomeprpsbtn")) {
        await this.click(".cmpwelcomeprpsbtn > a[aria-checked=true]", true);
        await this.click(".cmpboxbtnsave");
        return true;
      }
      await this.click(".cmpboxbtncustom");
      await this.waitForElement(".cmptblbox", 2e3);
      await this.click(".cmptdchoice > a[aria-checked=true]", true);
      await this.click(".cmpboxbtnyescustomchoices");
      this.hide("#cmpwrapper,#cmpbox", "display");
      return true;
    }
    async optIn() {
      if (this.apiAvailable) {
        return await this.mainWorldEval("EVAL_CONSENTMANAGER_4");
      }
      return await this.click(".cmpboxbtnyes");
    }
    async test() {
      if (this.apiAvailable) {
        return await this.mainWorldEval("EVAL_CONSENTMANAGER_5");
      }
      return false;
    }
  };

  // lib/cmps/evidon.ts
  var Evidon = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Evidon";
    }
    get hasSelfTest() {
      return false;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      return this.elementExists("#_evidon_banner, #_evidon-barrier-wrapper");
    }
    async detectPopup() {
      return this.elementVisible("#_evidon_banner, #_evidon-barrier-wrapper #_evidon-banner", "any");
    }
    async optOut() {
      if (await this.click("#_evidon-decline-button, #_evidon-barrier-declinebutton, #_evidon_banner #RejectLink")) {
        return true;
      }
      hideElements(getStyleElement(), "#evidon-prefdiag-overlay,#evidon-prefdiag-background,#_evidon-background");
      await this.waitForThenClick("#_evidon-option-button");
      await this.waitForElement("#evidon-prefdiag-overlay", 5e3);
      await this.wait(500);
      await this.waitForThenClick("#evidon-prefdiag-decline");
      return true;
    }
    async optIn() {
      return await this.click("#_evidon-accept-button, #_evidon-banner-acceptbutton");
    }
  };

  // lib/cmps/onetrust.ts
  var Onetrust = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Onetrust";
      this.prehideSelectors = ["#onetrust-banner-sdk,#onetrust-consent-sdk,.onetrust-pc-dark-filter,.js-consent-banner"];
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    // Some sites pre-render the banner before the SDK loads, and clicks before then may be lost.
    // Sites that copy the markup without the SDK never get #onetrust-style, so only wait when the page loads the SDK.
    async waitForSdk() {
      if (this.elementExists("#onetrust-style")) {
        return;
      }
      const sdkRef = /cookielaw\.org|otSDKStub|otBannerSdk/;
      const sdkExpected = Array.from(document.scripts).some((s) => sdkRef.test(s.src) || !s.src && sdkRef.test(s.textContent || ""));
      if (sdkExpected) {
        await this.waitForElement("#onetrust-style", 1e4);
      }
    }
    async detectCmp() {
      return this.elementExists("#onetrust-banner-sdk") || this.elementVisible("#onetrust-pc-sdk", "any");
    }
    async detectPopup() {
      return this.elementVisible("#onetrust-banner-sdk,#onetrust-pc-sdk", "any");
    }
    async optOut() {
      await this.wait(500);
      await this.waitForSdk();
      if (this.elementVisible("#onetrust-reject-all-handler", "any")) {
        return await this.click("#onetrust-reject-all-handler");
      }
      if (this.elementVisible(".ot-pc-refuse-all-handler", "any")) {
        return await this.click(".ot-pc-refuse-all-handler");
      }
      if (this.elementVisible(".js-reject-cookies", "any")) {
        return await this.click(".js-reject-cookies");
      }
      if (this.elementVisible(".onetrust-close-btn-handler", "any")) {
        const closeBtn = document.querySelector(".onetrust-close-btn-handler");
        const btnText = closeBtn?.textContent?.toLowerCase() || "";
        const rejectPatterns = ["without", "ohne", "sans", "sin ", "zonder", "senza", "refuse", "decline", "reject", "ablehnen"];
        if (rejectPatterns.some((pattern) => btnText.includes(pattern))) {
          return await this.click(".onetrust-close-btn-handler");
        }
        const banner = document.getElementById("onetrust-banner-sdk");
        const isCloseOnlyNotice = banner?.classList.contains("ot-close-btn-link") && !this.elementExists(
          "#onetrust-accept-btn-handler,#onetrust-reject-all-handler,#onetrust-pc-btn-handler,.ot-sdk-show-settings,button.js-cookie-settings"
        );
        if (isCloseOnlyNotice) {
          return await this.click(".onetrust-close-btn-handler");
        }
      }
      if (this.elementExists("#onetrust-pc-btn-handler")) {
        await this.click("#onetrust-pc-btn-handler");
      } else {
        await this.click(".ot-sdk-show-settings,button.js-cookie-settings");
      }
      await this.waitForElement("#onetrust-consent-sdk", 2e3);
      await this.wait(1e3);
      await this.click("#onetrust-consent-sdk input.category-switch-handler:checked,.js-editor-toggle-state:checked", true);
      await this.wait(1e3);
      await this.waitForElement(".save-preference-btn-handler,.js-consent-save", 2e3);
      await this.click(".save-preference-btn-handler,.js-consent-save");
      await this.waitForVisible("#onetrust-banner-sdk", 5e3, "none");
      return true;
    }
    async optIn() {
      await this.waitForSdk();
      return await this.click("#onetrust-accept-btn-handler,#accept-recommended-btn-handler,.js-accept-cookies");
    }
    async test() {
      return await waitFor(() => this.mainWorldEval("EVAL_ONETRUST_1"), 10, 500);
    }
  };

  // lib/cmps/klaro.ts
  var Klaro = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Klaro";
      this.prehideSelectors = [".klaro"];
      this.settingsOpen = false;
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      if (this.elementExists(".klaro > .cookie-modal")) {
        this.settingsOpen = true;
        return true;
      }
      return this.elementExists(".klaro > .cookie-notice");
    }
    async detectPopup() {
      return this.elementVisible(".klaro > .cookie-notice,.klaro > .cookie-modal", "any");
    }
    async optOut() {
      const apiOptOutSuccess = await this.mainWorldEval("EVAL_KLARO_TRY_API_OPT_OUT");
      if (apiOptOutSuccess) {
        return true;
      }
      if (await this.click(".klaro .cn-decline")) {
        return true;
      }
      await this.mainWorldEval("EVAL_KLARO_OPEN_POPUP");
      if (await this.click(".klaro .cn-decline")) {
        return true;
      }
      await this.click(
        ".cm-purpose:not(.cm-toggle-all) > input:not(.half-checked,.required,.only-required),.cm-purpose:not(.cm-toggle-all) > div > input:not(.half-checked,.required,.only-required)",
        true
      );
      return await this.click(".cm-btn-accept,.cm-button");
    }
    async optIn() {
      if (await this.click(".klaro .cm-btn-accept-all")) {
        return true;
      }
      if (this.settingsOpen) {
        await this.click(".cm-purpose:not(.cm-toggle-all) > input.half-checked", true);
        return await this.click(".cm-btn-accept");
      }
      return await this.click(".klaro .cookie-notice .cm-btn-success");
    }
    async test() {
      return await this.mainWorldEval("EVAL_KLARO_1");
    }
  };

  // lib/cmps/uniconsent.ts
  var Uniconsent = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Uniconsent";
    }
    get prehideSelectors() {
      return [".unic", ".modal:has(.unic)"];
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      return this.elementExists(".unic .unic-box,.unic .unic-bar,.unic .unic-modal");
    }
    async detectPopup() {
      return this.elementVisible(".unic .unic-box,.unic .unic-bar,.unic .unic-modal", "any");
    }
    async optOut() {
      await this.waitForElement(".unic button", 1e3);
      document.querySelectorAll(".unic button").forEach((button) => {
        const text = button.textContent || "";
        if (text.includes("Manage Options") || text.includes("Optionen verwalten")) {
          button.click();
        }
      });
      if (await this.waitForElement(".unic input[type=checkbox]", 1e3)) {
        await this.waitForElement(".unic button", 1e3);
        document.querySelectorAll(".unic input[type=checkbox]").forEach((c) => {
          if (c.checked) {
            c.click();
          }
        });
        for (const b of document.querySelectorAll(".unic button")) {
          const text = b.textContent || "";
          for (const pattern of ["Confirm Choices", "Save Choices", "Auswahl speichern"]) {
            if (text.includes(pattern)) {
              b.click();
              await this.wait(500);
              return true;
            }
          }
        }
      }
      return false;
    }
    async optIn() {
      return this.waitForThenClick(".unic #unic-agree");
    }
    async test() {
      await this.wait(1e3);
      const res = this.elementExists(".unic .unic-box,.unic .unic-bar");
      return !res;
    }
  };

  // lib/cmps/conversant.ts
  var Conversant = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.prehideSelectors = [".cmp-root"];
      this.name = "Conversant";
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      return this.elementExists(".cmp-root .cmp-receptacle");
    }
    async detectPopup() {
      return this.elementVisible(".cmp-root .cmp-receptacle", "any");
    }
    async optOut() {
      if (!await this.waitForThenClick(".cmp-main-button:not(.cmp-main-button--primary)")) {
        return false;
      }
      if (!await this.waitForElement(".cmp-view-tab-tabs")) {
        return false;
      }
      await this.waitForThenClick(".cmp-view-tab-tabs > :first-child");
      await this.waitForThenClick(".cmp-view-tab-tabs > .cmp-view-tab--active:first-child");
      for (const item of Array.from(document.querySelectorAll(".cmp-accordion-item"))) {
        item.querySelector(".cmp-accordion-item-title").click();
        await waitFor(() => !!item.querySelector(".cmp-accordion-item-content.cmp-active"), 10, 50);
        const content = item.querySelector(".cmp-accordion-item-content.cmp-active");
        if (!content) {
          return false;
        }
        content.querySelectorAll(".cmp-toggle-actions .cmp-toggle-deny:not(.cmp-toggle-deny--active)").forEach((e) => e.click());
        content.querySelectorAll(".cmp-toggle-actions .cmp-toggle-checkbox:not(.cmp-toggle-checkbox--active)").forEach((e) => e.click());
      }
      await this.click(".cmp-main-button:not(.cmp-main-button--primary)");
      return true;
    }
    async optIn() {
      return this.waitForThenClick(".cmp-main-button.cmp-main-button--primary");
    }
    async test() {
      return document.cookie.includes("cmp-data=0");
    }
  };

  // lib/cmps/tiktok.ts
  var Tiktok = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "tiktok.com";
    }
    get hasSelfTest() {
      return true;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    getShadowRoot() {
      const container = document.querySelector("tiktok-cookie-banner");
      if (!container) {
        return null;
      }
      return container.shadowRoot;
    }
    async detectCmp() {
      return this.elementExists("tiktok-cookie-banner");
    }
    async detectPopup() {
      const banner = this.getShadowRoot()?.querySelector(".tiktok-cookie-banner");
      return isElementVisible(banner);
    }
    async optOut() {
      const logsConfig = this.autoconsent.config.logs;
      const declineButton = this.getShadowRoot()?.querySelector(".button-wrapper button:first-child");
      if (declineButton) {
        logsConfig.rulesteps && console.log("[clicking]", declineButton);
        declineButton.click();
        return true;
      } else {
        logsConfig.errors && console.log("no decline button found");
        return false;
      }
    }
    async optIn() {
      const logsConfig = this.autoconsent.config.logs;
      const acceptButton = this.getShadowRoot()?.querySelector(".button-wrapper button:last-child");
      if (acceptButton) {
        logsConfig.rulesteps && console.log("[clicking]", acceptButton);
        acceptButton.click();
        return true;
      } else {
        logsConfig.errors && console.log("no accept button found");
        return false;
      }
    }
    async test() {
      const match = document.cookie.match(/cookie-consent=([^;]+)/);
      if (!match) {
        return false;
      }
      const value = JSON.parse(decodeURIComponent(match[1]));
      return Object.values(value).every((x) => typeof x !== "boolean" || x === false);
    }
  };

  // lib/cmps/admiral.ts
  var Admiral = class extends AutoConsentCMPBase {
    constructor() {
      super(...arguments);
      this.name = "Admiral";
      this.consentCardSelector = "div > div[class*=Card] > div[class*=Frame] > div[class*=Pills] > button[class*=Pills__StyledPill]";
    }
    getVrmNotice() {
      return Array.from(document.querySelectorAll("body > div")).find((element) => {
        const text = element.innerText || "";
        const rect = element.getBoundingClientRect();
        const style = getComputedStyle(element);
        return style.position === "fixed" && rect.width > 0 && rect.height > 0 && text.includes("Your Privacy") && text.includes("VRM") && text.includes("Admiral");
      });
    }
    isVisibleElement(element) {
      const rect = element.getBoundingClientRect();
      const style = getComputedStyle(element);
      return rect.width > 0 && rect.height > 0 && style.display !== "none" && style.visibility !== "hidden";
    }
    getVrmCloseButton() {
      const notice = this.getVrmNotice();
      const closeButton = notice?.querySelector('button[aria-label="Close"]');
      return closeButton && this.isVisibleElement(closeButton) ? closeButton : null;
    }
    get hasSelfTest() {
      return false;
    }
    get isIntermediate() {
      return false;
    }
    get isCosmetic() {
      return false;
    }
    async detectCmp() {
      return await this.elementExists(this.consentCardSelector) || Boolean(this.getVrmNotice());
    }
    async detectPopup() {
      return await this.elementVisible(this.consentCardSelector, "any") || Boolean(this.getVrmNotice());
    }
    async optOut() {
      const rejectAllSelector = "xpath///button[contains(., 'Afvis alle') or contains(., 'Reject all') or contains(., 'Odbaci sve') or contains(., 'Rechazar todo') or contains(., 'Atmesti visus') or contains(., 'Odm\xEDtnout v\u0161e') or contains(., '\u0391\u03C0\u03CC\u03C1\u03C1\u03B9\u03C8\u03B7 \u03CC\u03BB\u03C9\u03BD') or contains(., 'Rejeitar tudo') or contains(., 'T\xFCm\xFCn\xFC reddet') or contains(., '\u041E\u0442\u043A\u043B\u043E\u043D\u0438\u0442\u044C \u0432\u0441\u0435') or contains(., 'Noraid\u012Bt visu') or contains(., 'Avvisa alla') or contains(., 'Odrzu\u0107 wszystkie') or contains(., 'Alles afwijzen') or contains(., '\u041E\u0442\u0445\u0432\u044A\u0440\u043B\u044F\u043D\u0435 \u043D\u0430 \u0432\u0441\u0438\u0447\u043A\u0438') or contains(., 'Rifiuta tutto') or contains(., 'Zavrni vse') or contains(., 'Az \xF6sszes elutas\xEDt\xE1sa') or contains(., 'Respinge\u021Bi tot') or contains(., 'Alles ablehnen') or contains(., 'Tout rejeter') or contains(., 'Odmietnu\u0165 v\u0161etko') or contains(., 'L\xFCkka k\xF5ik tagasi') or contains(., 'Hylk\xE4\xE4 kaikki')]";
      if (await this.waitForElement(rejectAllSelector, 500)) {
        const rejectButton = this.elementSelector(rejectAllSelector)[0];
        if (rejectButton.disabled) {
          const cardBody = rejectButton.closest("div[class*=CardHeader]")?.parentElement?.querySelector("div[class*=CardBody]");
          if (cardBody) {
            cardBody.scrollTop = cardBody.scrollHeight;
          }
          await waitFor(() => !rejectButton.disabled, 10, 200);
        }
        return await this.click(rejectAllSelector);
      }
      const vrmCloseButton = this.getVrmCloseButton();
      if (vrmCloseButton) {
        return await this.clickElement(vrmCloseButton);
      }
      const purposesButtonSelector = "xpath///button[contains(., 'Zwecke') or contains(., '\u03A3\u03BA\u03BF\u03C0\u03BF\u03AF') or contains(., 'Purposes') or contains(., '\u0426\u0435\u043B\u0438') or contains(., 'Eesm\xE4rgid') or contains(., 'Tikslai') or contains(., 'Svrhe') or contains(., 'Cele') or contains(., '\xDA\u010Dely') or contains(., 'Finalidades') or contains(., 'M\u0113r\u0137i') or contains(., 'Scopuri') or contains(., 'Fines') or contains(., '\xC4ndam\xE5l') or contains(., 'Finalit\xE9s') or contains(., 'Doeleinden') or contains(., 'Tarkoitukset') or contains(., 'Scopi') or contains(., 'Ama\xE7lar') or contains(., 'Nameni') or contains(., 'C\xE9lok') or contains(., 'Form\xE5l')]";
      const saveAndExitSelector = "xpath///button[contains(., 'Spara & avsluta') or contains(., 'Save & exit') or contains(., 'Ulo\u017Eit a ukon\u010Dit') or contains(., 'Enregistrer et quitter') or contains(., 'Speichern & Verlassen') or contains(., 'Tallenna ja poistu') or contains(., 'I\u0161saugoti ir i\u0161eiti') or contains(., 'Opslaan & afsluiten') or contains(., 'Guardar y salir') or contains(., 'Shrani in zapri') or contains(., 'Ulo\u017Ei\u0165 a ukon\u010Di\u0165') or contains(., 'Kaydet ve \xE7\u0131k\u0131\u015F yap') or contains(., '\u0421\u043E\u0445\u0440\u0430\u043D\u0438\u0442\u044C \u0438 \u0432\u044B\u0439\u0442\u0438') or contains(., 'Salvesta ja v\xE4lju') or contains(., 'Salva ed esci') or contains(., 'Gem & afslut') or contains(., '\u0391\u03C0\u03BF\u03B8\u03AE\u03BA\u03B5\u03C5\u03C3\u03B7 \u03BA\u03B1\u03B9 \u03AD\u03BE\u03BF\u03B4\u03BF\u03C2') or contains(., 'Saglab\u0101t un iziet') or contains(., 'Ment\xE9s \xE9s kil\xE9p\xE9s') or contains(., 'Guardar e sair') or contains(., 'Zapisz & zako\u0144cz') or contains(., 'Salvare \u0219i ie\u0219ire') or contains(., 'Spremi i iza\u0111i') or contains(., '\u0417\u0430\u043F\u0430\u0437\u0432\u0430\u043D\u0435 \u0438 \u0438\u0437\u0445\u043E\u0434')]";
      if (await this.waitForThenClick(purposesButtonSelector) && await this.waitForVisible(saveAndExitSelector)) {
        const popupBody = this.elementSelector(saveAndExitSelector)[0].parentElement?.parentElement;
        const checkboxes = popupBody?.querySelectorAll("input[type=checkbox]:checked");
        checkboxes?.forEach((checkbox) => checkbox.click());
        return await this.click(saveAndExitSelector);
      }
      return false;
    }
    async optIn() {
      return await this.click(
        "xpath///button[contains(., 'Sprejmi vse') or contains(., 'Prihvati sve') or contains(., 'Godk\xE4nn alla') or contains(., 'Prija\u0165 v\u0161etko') or contains(., '\u041F\u0440\u0438\u043D\u044F\u0442\u044C \u0432\u0441\u0435') or contains(., 'Aceptar todo') or contains(., '\u0391\u03C0\u03BF\u03B4\u03BF\u03C7\u03AE \u03CC\u03BB\u03C9\u03BD') or contains(., 'Zaakceptuj wszystkie') or contains(., 'Accetta tutto') or contains(., 'Priimti visus') or contains(., 'Pie\u0146emt visu') or contains(., 'T\xFCm\xFCn\xFC kabul et') or contains(., 'Az \xF6sszes elfogad\xE1sa') or contains(., 'Accept all') or contains(., '\u041F\u0440\u0438\u0435\u043C\u0430\u043D\u0435 \u043D\u0430 \u0432\u0441\u0438\u0447\u043A\u0438') or contains(., 'Accepter alle') or contains(., 'Hyv\xE4ksy kaikki') or contains(., 'Tout accepter') or contains(., 'Alles accepteren') or contains(., 'Aktsepteeri k\xF5ik') or contains(., 'P\u0159ijmout v\u0161e') or contains(., 'Alles akzeptieren') or contains(., 'Aceitar tudo') or contains(., 'Accepta\u021Bi tot')]"
      );
    }
  };

  // lib/cmps/all.ts
  var dynamicCMPs = [
    TrustArcTop,
    Cookiebot,
    SourcePoint,
    ConsentManager,
    Evidon,
    Onetrust,
    Klaro,
    Uniconsent,
    Conversant,
    Tiktok,
    Admiral
  ];

  // lib/dom-actions.ts
  var DEFAULT_CLICK_RETRY_INTERVAL = 300;
  var CLICK_RETRY_POLL_INTERVAL = 50;
  var DomActions = class {
    constructor(autoconsentInstance) {
      this.autoconsentInstance = autoconsentInstance;
    }
    async clickElement(element) {
      if (!element || !(element instanceof HTMLElement)) {
        return false;
      }
      this.autoconsentInstance.config.logs.rulesteps && console.log("[clickElement]", element);
      element.click();
      return true;
    }
    async click(selector, all = false) {
      const elements = this.elementSelector(selector);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[click]", selector, all, elements);
      if (elements.length > 0) {
        if (all) {
          for (const e of elements) {
            e.click();
            await new Promise((resolve) => setTimeout(resolve, 0));
          }
        } else {
          elements[0].click();
        }
      }
      return elements.length > 0;
    }
    elementExists(selector) {
      const exists = this.elementSelector(selector).length > 0;
      return exists;
    }
    elementVisible(selector, check = "all") {
      const elem = this.elementSelector(selector);
      const results = new Array(elem.length);
      elem.forEach((e, i) => {
        results[i] = isElementVisible(e);
      });
      if (check === "none") {
        return results.every((r) => !r);
      } else if (results.length === 0) {
        return false;
      } else if (check === "any") {
        return results.some((r) => r);
      }
      return results.every((r) => r);
    }
    waitForElement(selector, timeout = 1e4) {
      const interval = 200;
      const times = Math.ceil(timeout / interval);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[waitForElement]", selector);
      return waitFor(() => this.elementSelector(selector).length > 0, times, interval);
    }
    waitForVisible(selector, timeout = 1e4, check = "any") {
      const interval = 200;
      const times = Math.ceil(timeout / interval);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[waitForVisible]", selector);
      return waitFor(() => this.elementVisible(selector, check), times, interval);
    }
    async waitForThenClick(selector, timeout = 1e4, all = false, retries = 0, retryInterval = DEFAULT_CLICK_RETRY_INTERVAL) {
      await this.waitForElement(selector, timeout);
      let clicked = await this.click(selector, all);
      for (let attempt = 0; clicked && attempt < retries; attempt++) {
        const pollTimes = Math.ceil(retryInterval / CLICK_RETRY_POLL_INTERVAL);
        const isGone = await waitFor(() => !this.elementVisible(selector, "any"), pollTimes, CLICK_RETRY_POLL_INTERVAL);
        if (isGone) {
          break;
        }
        this.autoconsentInstance.config.logs.rulesteps && console.log("[waitForThenClick] retrying click", selector);
        clicked = await this.click(selector, all);
      }
      return clicked;
    }
    wait(ms) {
      this.autoconsentInstance.config.logs.rulesteps && this.autoconsentInstance.config.logs.waits && console.log("[wait]", ms);
      return new Promise((resolve) => {
        setTimeout(() => {
          resolve(true);
        }, ms);
      });
    }
    cookieContains(substring) {
      return document.cookie.includes(substring);
    }
    hide(selector, method) {
      this.autoconsentInstance.config.logs.rulesteps && console.log("[hide]", selector);
      const styleEl = getStyleElement();
      return hideElements(styleEl, selector, method);
    }
    stylesheet(cssRule, stylesheetId) {
      this.autoconsentInstance.config.logs.rulesteps && console.log("[stylesheet]", cssRule, stylesheetId);
      const styleEl = getStyleElement();
      return appendStylesheetRule(styleEl, cssRule, stylesheetId);
    }
    removeClass(selector, className) {
      const elements = this.elementSelector(selector);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[removeClass]", selector, className, elements);
      elements.forEach((el) => el.classList.remove(className));
      return elements.length > 0;
    }
    setStyle(selector, css) {
      const elements = this.elementSelector(selector);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[setStyle]", selector, css, elements);
      elements.forEach((el) => el.style.cssText = css);
      return elements.length > 0;
    }
    addStyle(selector, css) {
      const elements = this.elementSelector(selector);
      this.autoconsentInstance.config.logs.rulesteps && console.log("[addStyle]", selector, css, elements);
      elements.forEach((el) => el.style.cssText += "; " + css);
      return elements.length > 0;
    }
    prehide(selector) {
      const styleEl = getStyleElement("autoconsent-prehide");
      this.autoconsentInstance.config.logs.lifecycle && console.log("[prehide]", styleEl, location.href);
      return hideElements(styleEl, selector, "opacity");
    }
    undoPrehide() {
      const existingElement = getStyleElement("autoconsent-prehide");
      this.autoconsentInstance.config.logs.lifecycle && console.log("[undoprehide]", existingElement, location.href);
      existingElement.remove();
    }
    async createOrUpdateStyleSheet(cssText, styleSheet) {
      if (!styleSheet) {
        styleSheet = new CSSStyleSheet();
      }
      styleSheet = await styleSheet.replace(cssText);
      return styleSheet;
    }
    removeStyleSheet(styleSheet) {
      if (styleSheet) {
        styleSheet.replace("");
        return true;
      }
      return false;
    }
    querySingleReplySelector(selector, parent = document) {
      if (selector.startsWith("aria/")) {
        return [];
      }
      if (selector.startsWith("xpath/")) {
        const xpath = selector.slice(6);
        const result = document.evaluate(xpath, parent, null, XPathResult.ANY_TYPE, null);
        let node;
        const elements = [];
        while (node = result.iterateNext()) {
          elements.push(node);
        }
        return elements;
      }
      if (selector.startsWith("text/")) {
        return [];
      }
      if (selector.startsWith("pierce/")) {
        return [];
      }
      if (parent.shadowRoot) {
        return Array.from(parent.shadowRoot.querySelectorAll(selector));
      }
      if (parent.contentDocument?.querySelectorAll) {
        return Array.from(parent.contentDocument.querySelectorAll(selector));
      }
      return Array.from(parent.querySelectorAll(selector));
    }
    querySelectorChain(selectors) {
      let parent = document;
      let matches = [];
      for (const selector of selectors) {
        matches = this.querySingleReplySelector(selector, parent);
        if (matches.length === 0) {
          return [];
        }
        parent = matches[0];
      }
      return matches;
    }
    elementSelector(selector) {
      if (typeof selector === "string") {
        return this.querySingleReplySelector(selector);
      }
      return this.querySelectorChain(selector);
    }
    waitForMutation(selector, timeout = 6e4) {
      const node = this.elementSelector(selector);
      if (node.length === 0) {
        throw new Error(`${selector} did not match any elements`);
      }
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => {
          reject(new Error("Timed out waiting for mutation"));
          observer.disconnect();
        }, timeout);
        const observer = new MutationObserver(() => {
          clearTimeout(timer);
          observer.disconnect();
          resolve(true);
        });
        observer.observe(node[0], {
          subtree: true,
          childList: true,
          attributes: true
        });
      });
    }
  };

  // lib/encoding.ts
  var compactedRuleSteps = [
    ["exists", "e"],
    ["visible", "v"],
    ["waitForThenClick", "c"],
    ["click", "k"],
    ["waitFor", "w"],
    ["waitForVisible", "wv"],
    ["hide", "h"],
    ["cookieContains", "cc"]
  ];
  function decodeNullableBoolean(value) {
    if (value === 1) {
      return true;
    }
    if (value === 0) {
      return false;
    }
    return void 0;
  }
  function decodeRules(encoded) {
    if (encoded.v > 1) {
      throw new Error("Unsupported rule format.");
    }
    return encoded.r.filter((r) => r[0] <= SUPPORTED_RULE_STEP_VERSION).map((rule) => new CompactedCMPRule(rule, encoded.s));
  }
  var CompactedCMPRule = class {
    constructor(rule, strings) {
      this.intermediate = false;
      this.optIn = [];
      this.r = rule;
      this.s = strings;
      if (this.r[10] && this.r[10].intermediate) {
        this.intermediate = this.r[10].intermediate;
      }
    }
    _decodeRuleStep(step) {
      const clonedStep = { ...step };
      const decodeRuleStep = this._decodeRuleStep.bind(this);
      for (const [longKey, shortKey] of compactedRuleSteps) {
        if (clonedStep[shortKey] !== void 0) {
          clonedStep[longKey] = this.s[clonedStep[shortKey]];
          delete clonedStep[shortKey];
        }
      }
      if (step.if) {
        clonedStep.if = decodeRuleStep(step.if);
        clonedStep.then = step.then && step.then.map(decodeRuleStep);
        if (step.else) {
          clonedStep.else = step.else.map(decodeRuleStep);
        }
      }
      if (step.any) {
        clonedStep.any = step.any.map(decodeRuleStep);
      }
      return { ...clonedStep };
    }
    get minimumRuleStepVersion() {
      return this.r[0];
    }
    get name() {
      return this.r[1];
    }
    get cosmetic() {
      return decodeNullableBoolean(this.r[2]);
    }
    get runContext() {
      const runContext = {};
      const urlPattern = this.r[3];
      const mainFrame = this.r[4];
      const runInMainFrame = decodeNullableBoolean(Math.floor(mainFrame / 10) % 10);
      const runInSubFrame = decodeNullableBoolean(mainFrame % 10);
      if (runInMainFrame !== void 0) {
        runContext.main = runInMainFrame;
      }
      if (runInSubFrame !== void 0) {
        runContext.frame = runInSubFrame;
      }
      if (urlPattern !== "") {
        runContext.urlPattern = urlPattern;
      }
      return runContext;
    }
    get prehideSelectors() {
      return this.r[5].map((i) => this.s[i].toString());
    }
    get detectCmp() {
      return this.r[6].map(this._decodeRuleStep.bind(this));
    }
    get detectPopup() {
      return this.r[7].map(this._decodeRuleStep.bind(this));
    }
    get optOut() {
      return this.r[8].map(this._decodeRuleStep.bind(this));
    }
    get test() {
      return this.r[9].map(this._decodeRuleStep.bind(this));
    }
  };
  function clearUnusedStrings(ruleset, ignoreBeforeIndex = 0) {
    const { v, s, r } = ruleset;
    const usedStringIds = /* @__PURE__ */ new Set();
    function addStringIdsFromRuleSteps(steps) {
      steps.forEach((step) => {
        for (const [, shortKey] of compactedRuleSteps) {
          if (step[shortKey] !== void 0) {
            usedStringIds.add(step[shortKey]);
          }
        }
        if (step.if) {
          addStringIdsFromRuleSteps([step.if]);
        }
        if (step.then) {
          addStringIdsFromRuleSteps(step.then);
        }
        if (step.else) {
          addStringIdsFromRuleSteps(step.else);
        }
        if (step.any) {
          addStringIdsFromRuleSteps(step.any);
        }
      });
    }
    ruleset.r.forEach((rule) => {
      addStringIdsFromRuleSteps(rule[6]);
      addStringIdsFromRuleSteps(rule[7]);
      addStringIdsFromRuleSteps(rule[8]);
      addStringIdsFromRuleSteps(rule[9]);
      rule[5].forEach((id) => usedStringIds.add(id));
    });
    return {
      v,
      r,
      s: s.slice(0, Math.max(...usedStringIds) + 1).map((str, idx) => {
        if (idx < ignoreBeforeIndex || usedStringIds.has(idx)) {
          return str;
        }
        return "";
      })
    };
  }
  function shouldRunRuleInContext(rule, mainFrame, url) {
    const runContext = rule[4];
    if (mainFrame && runContext === 1) {
      return false;
    }
    if (!mainFrame && [20, 22, 10, 12].includes(runContext)) {
      return false;
    }
    const urlPattern = rule[3];
    if (urlPattern && urlPattern !== "" && url.match(urlPattern) === null) {
      return false;
    }
    return true;
  }
  function filterCompactRules(rules, context) {
    const { v, s, r, index: index2 } = rules;
    const { url, mainFrame } = context;
    const shouldRunInContext = (rule) => shouldRunRuleInContext(rule, mainFrame, url);
    if (!mainFrame) {
      const ruleset = {
        v,
        s: s.slice(0, index2.frameStringEnd),
        r: r.slice(index2.frameRuleRange[0], index2.frameRuleRange[1]).filter(shouldRunInContext)
      };
      return clearUnusedStrings(ruleset);
    }
    const genericRules = r.slice(index2.genericRuleRange[0], index2.genericRuleRange[1]);
    const specificRules = r.slice(index2.specificRuleRange[0], index2.specificRuleRange[1]).filter(shouldRunInContext);
    if (specificRules.length > 0) {
      const ruleset = {
        v,
        s,
        r: [...genericRules, ...specificRules]
      };
      return clearUnusedStrings(ruleset);
    }
    return {
      v,
      s: s.slice(0, index2.genericStringEnd),
      r: genericRules
    };
  }

  // lib/web.ts
  function filterCMPs(rules, config) {
    return rules.filter((cmp) => {
      return (!config.disabledCmps || !config.disabledCmps.includes(cmp.name)) && // CMP is not disabled
      (config.enableCosmeticRules || !cmp.isCosmetic) && // CMP is not cosmetic or cosmetic rules are enabled
      (config.enableGeneratedRules || !cmp.name.startsWith("auto_"));
    });
  }
  var _config;
  var AutoConsent = class {
    constructor(sendContentMessage, config = null, declarativeRules = null) {
      this.id = getRandomID();
      this.rules = [];
      __privateAdd(this, _config);
      this.state = {
        lifecycle: "loading",
        prehideOn: false,
        findCmpAttempts: 0,
        detectedCmps: [],
        detectedPopups: [],
        heuristicPatterns: [],
        heuristicSnippets: [],
        selfTest: null,
        clicks: 0,
        startTime: 0,
        endTime: 0
      };
      evalState.sendContentMessage = sendContentMessage;
      this.sendContentMessage = sendContentMessage;
      this.rules = [];
      this.updateState({ lifecycle: "loading" });
      this.addDynamicRules();
      if (config) {
        this.initialize(config, declarativeRules);
      } else {
        if (declarativeRules) {
          this.parseDeclarativeRules(declarativeRules);
        }
        const initMsg = {
          type: "init",
          url: window.location.href
        };
        sendContentMessage(initMsg);
        this.updateState({ lifecycle: "waitingForInitResponse" });
      }
      this.domActions = new DomActions(this);
    }
    get config() {
      if (!__privateGet(this, _config)) {
        throw new Error("AutoConsent is not initialized yet");
      }
      return __privateGet(this, _config);
    }
    initialize(config, declarativeRules) {
      const normalizedConfig = normalizeConfig(config);
      normalizedConfig.logs.lifecycle && console.log("autoconsent init", window.location.href);
      __privateSet(this, _config, normalizedConfig);
      if (!normalizedConfig.enabled) {
        normalizedConfig.logs.lifecycle && console.log("autoconsent is disabled");
        return;
      }
      if (declarativeRules) {
        this.parseDeclarativeRules(declarativeRules);
      }
      this.rules = filterCMPs(this.rules, normalizedConfig);
      if (this.shouldPrehide) {
        if (document.documentElement) {
          this.prehideElements();
        } else {
          const delayedPrehide = () => {
            window.removeEventListener("DOMContentLoaded", delayedPrehide);
            this.prehideElements();
          };
          window.addEventListener("DOMContentLoaded", delayedPrehide);
        }
      }
      if (document.readyState === "loading") {
        const onReady = () => {
          window.removeEventListener("DOMContentLoaded", onReady);
          this.start();
        };
        window.addEventListener("DOMContentLoaded", onReady);
      } else {
        this.start();
      }
      this.updateState({ lifecycle: "initialized" });
    }
    get shouldPrehide() {
      return this.config.enablePrehide && !this.config.visualTest;
    }
    saveFocus() {
      this.focusedElement = document.activeElement;
      if (this.focusedElement) {
        this.config.logs.lifecycle && console.log("saving focus", this.focusedElement, location.href);
      }
    }
    restoreFocus() {
      if (this.focusedElement) {
        this.config.logs.lifecycle && console.log("restoring focus", this.focusedElement, location.href);
        try {
          this.focusedElement.focus({ preventScroll: true });
        } catch (e) {
          this.config.logs.errors && console.warn("error restoring focus", e);
        }
        this.focusedElement = void 0;
      }
    }
    addDynamicRules() {
      dynamicCMPs.forEach((Cmp) => {
        this.rules.push(new Cmp(this));
      });
    }
    parseDeclarativeRules(declarativeRules) {
      const perfEnabled = __privateGet(this, _config)?.performanceLoggingEnabled;
      perfEnabled && performance.mark("parseDeclarativeRulesStart");
      if (declarativeRules.autoconsent) {
        declarativeRules.autoconsent.forEach((ruleset) => {
          this.addDeclarativeCMP(ruleset);
        });
      }
      if (declarativeRules.compact) {
        try {
          const rules = decodeRules(declarativeRules.compact);
          rules.forEach(this.addDeclarativeCMP.bind(this));
        } catch (e) {
          __privateGet(this, _config)?.logs.errors && console.error(e);
        }
      }
      perfEnabled && performance.mark("parseDeclarativeRulesEnd");
      perfEnabled && performance.measure("parseDeclarativeRules", "parseDeclarativeRulesStart", "parseDeclarativeRulesEnd");
    }
    addDeclarativeCMP(ruleset) {
      if ((ruleset.minimumRuleStepVersion || 1) <= SUPPORTED_RULE_STEP_VERSION) {
        this.rules.push(new AutoConsentCMP(ruleset, this));
      }
    }
    // start the detection process, possibly with a delay
    start() {
      scheduleWhenIdle(() => this._start());
    }
    async _start() {
      const logsConfig = this.config.logs;
      logsConfig.lifecycle && console.log(`Detecting CMPs on ${window.location.href}`);
      this.updateState({ lifecycle: "started" });
      const foundCmps = await this.findCmp(this.config.detectRetries);
      this.updateState({ detectedCmps: foundCmps.map((c) => c.name) });
      if (this.config.performanceLoggingEnabled) {
        this.updateState({ performance: this.measurePerformance() });
      }
      if (foundCmps.length === 0) {
        logsConfig.lifecycle && console.log("no CMP found", location.href);
        if (this.shouldPrehide) {
          this.undoPrehide();
        }
        this.updateState({ lifecycle: "nothingDetected" });
        return false;
      }
      this.updateState({ lifecycle: "cmpDetected" });
      const staticCmps = [];
      const cosmeticCmps = [];
      for (const cmp of foundCmps) {
        if (cmp.isCosmetic) {
          cosmeticCmps.push(cmp);
        } else {
          staticCmps.push(cmp);
        }
      }
      let result = false;
      let foundPopups = await this.detectPopups(staticCmps, async (cmp) => {
        result = await this.handlePopup(cmp);
      });
      if (foundPopups.length === 0) {
        foundPopups = await this.detectPopups(cosmeticCmps, async (cmp) => {
          result = await this.handlePopup(cmp);
        });
      }
      if (foundPopups.length === 0) {
        logsConfig.lifecycle && console.log("no popup found");
        if (this.shouldPrehide) {
          this.undoPrehide();
        }
        return false;
      }
      if (foundPopups.length > 1) {
        const errorDetails = {
          msg: `Found multiple CMPs, check the detection rules.`,
          cmps: foundPopups.map((cmp) => cmp.name)
        };
        logsConfig.errors && console.warn(errorDetails.msg, errorDetails.cmps);
        this.sendContentMessage({
          type: "autoconsentError",
          details: errorDetails
        });
      }
      return result;
    }
    async findCmp(retries) {
      const logsConfig = this.config.logs;
      this.updateState({ findCmpAttempts: this.state.findCmpAttempts + 1 });
      const foundCMPs = [];
      const isTop = isTopFrame();
      const siteSpecificRules = [];
      const genericRules = [];
      this.rules.forEach((cmp) => {
        if (cmp.checkFrameContext(isTop)) {
          const isSiteSpecific = !!cmp.runContext.urlPattern;
          if (cmp.hasMatchingUrlPattern()) {
            siteSpecificRules.push(cmp);
          } else if (!isSiteSpecific) {
            genericRules.push(cmp);
          }
        }
      });
      const heuristicEnabled = isTop && this.config.heuristicMode !== "off";
      const heuristicRules = heuristicEnabled && this.state.findCmpAttempts % 2 === 0 ? [new AutoConsentHeuristicCMP(this, this.config.heuristicMode)] : [];
      const rulesPriorityStages = [
        ["site-specific", siteSpecificRules],
        ["generic", genericRules],
        ["heuristic", heuristicRules]
      ];
      const runDetectCmp = async (cmp) => {
        try {
          const result = await cmp.detectCmp();
          if (result) {
            logsConfig.lifecycle && console.log(`Found CMP: ${cmp.name} ${window.location.href}`);
            this.sendContentMessage({
              type: "cmpDetected",
              url: location.href,
              cmp: cmp.name
            });
            foundCMPs.push(cmp);
          }
        } catch (e) {
          logsConfig.errors && console.warn(`error detecting ${cmp.name}`, e);
        }
      };
      const mutationObserver = this.domActions.waitForMutation("html");
      mutationObserver.catch(() => {
      });
      for (const [stageName, ruleGroup] of rulesPriorityStages) {
        logsConfig.lifecycle && ruleGroup.length > 0 && console.log(
          `Trying ${stageName} rules`,
          ruleGroup.map((r) => r.name)
        );
        this.config.performanceLoggingEnabled && performance.mark(`findCmpStage_${stageName}`);
        await Promise.all(ruleGroup.map(runDetectCmp));
        this.config.performanceLoggingEnabled && performance.mark(`findCmpStageEnd_${stageName}`);
        this.config.performanceLoggingEnabled && performance.measure(`findCmp_${stageName}`, `findCmpStage_${stageName}`, `findCmpStageEnd_${stageName}`);
        if (foundCMPs.length > 0) {
          break;
        }
      }
      this.detectHeuristics();
      if (foundCMPs.length === 0 && retries > 0) {
        const waitFor2 = [this.domActions.wait(500)];
        const heuristicSkipped = heuristicEnabled && heuristicRules.length === 0;
        if (this.state.findCmpAttempts > 1 && !heuristicSkipped) {
          waitFor2.push(mutationObserver);
        }
        try {
          await Promise.all(waitFor2);
        } catch (e) {
          return [];
        }
        return this.findCmp(retries - 1);
      }
      return foundCMPs;
    }
    detectHeuristics() {
      if (this.config.enableHeuristicDetection) {
        this.config.performanceLoggingEnabled && performance.mark("detectHeuristicsStart");
        const { patterns, snippets: snippets2 } = checkHeuristicPatterns(document.documentElement?.innerText || "", "page");
        if (patterns.length > 0 && (patterns.length !== this.state.heuristicPatterns.length || this.state.heuristicPatterns.some((p, i) => p !== patterns[i]))) {
          this.config.logs.lifecycle && console.log("Heuristic patterns found", patterns, snippets2);
          this.updateState({ heuristicPatterns: patterns, heuristicSnippets: snippets2 });
        }
        this.config.performanceLoggingEnabled && performance.mark("detectHeuristicsEnd");
        this.config.performanceLoggingEnabled && performance.measure("detectHeuristics", "detectHeuristicsStart", "detectHeuristicsEnd");
      }
    }
    /**
     * Detect if a CMP has a popup open. Fullfils with the CMP if a popup is open, otherwise rejects.
     */
    async detectPopup(cmp) {
      const isOpen = await this.waitForPopup(cmp).catch((error) => {
        this.config.logs.errors && console.warn(`error waiting for a popup for ${cmp.name}`, error);
        return false;
      });
      if (isOpen) {
        this.updateState({ detectedPopups: this.state.detectedPopups.concat([cmp.name]) });
        this.sendContentMessage({
          type: "popupFound",
          cmp: cmp.name,
          url: location.href
        });
        return cmp;
      }
      throw new Error("Popup is not shown");
    }
    /**
     * Detect if any of the CMPs has a popup open. Returns a list of CMPs with open popups.
     */
    async detectPopups(cmps, onFirstPopupAppears) {
      const tasks = cmps.map((cmp) => this.detectPopup(cmp));
      await Promise.any(tasks).then((cmp) => {
        this.detectHeuristics();
        onFirstPopupAppears(cmp);
      }).catch(() => {
      });
      const results = await Promise.allSettled(tasks);
      const popups = [];
      for (const result of results) {
        if (result.status === "fulfilled") {
          popups.push(result.value);
        }
      }
      return popups;
    }
    async handlePopup(cmp) {
      this.updateState({ lifecycle: "openPopupDetected", startTime: Date.now() });
      if (this.shouldPrehide && !this.state.prehideOn) {
        this.prehideElements();
      }
      this.foundCmp = cmp;
      if (this.config.autoAction === "optOut") {
        return await this.doOptOut();
      } else if (this.config.autoAction === "optIn") {
        return await this.doOptIn();
      } else {
        this.config.logs.lifecycle && console.log("waiting for opt-out signal...", location.href);
        return true;
      }
    }
    async doOptOut() {
      const logsConfig = this.config.logs;
      this.updateState({ lifecycle: "runningOptOut" });
      this.saveFocus();
      let optOutResult;
      if (!this.foundCmp) {
        logsConfig.errors && console.log("no CMP to opt out");
        optOutResult = false;
      } else {
        logsConfig.lifecycle && console.log(`CMP ${this.foundCmp.name}: opt out on ${window.location.href}`);
        optOutResult = await this.foundCmp.optOut();
        logsConfig.lifecycle && console.log(`${this.foundCmp.name}: opt out result ${optOutResult}`);
      }
      if (this.shouldPrehide) {
        this.undoPrehide();
      }
      this.sendContentMessage({
        type: "optOutResult",
        cmp: this.foundCmp ? this.foundCmp.name : "none",
        result: optOutResult,
        scheduleSelfTest: Boolean(this.foundCmp && this.foundCmp.hasSelfTest),
        url: location.href
      });
      if (optOutResult && this.foundCmp && !this.foundCmp.isIntermediate) {
        this.state.endTime = Date.now();
        logsConfig.lifecycle && console.log(
          `${this.foundCmp.name}: done in ${this.state.endTime - this.state.startTime}ms with ${this.state.clicks} clicks`
        );
        this.sendContentMessage({
          type: "autoconsentDone",
          cmp: this.foundCmp?.name,
          isCosmetic: this.foundCmp?.isCosmetic,
          url: location.href,
          duration: this.state.endTime - this.state.startTime,
          totalClicks: this.state.clicks
        });
        this.updateState({ lifecycle: "done" });
      } else {
        this.updateState({ lifecycle: optOutResult ? "optOutSucceeded" : "optOutFailed" });
      }
      this.restoreFocus();
      return optOutResult;
    }
    async doOptIn() {
      const logsConfig = this.config.logs;
      this.updateState({ lifecycle: "runningOptIn" });
      this.saveFocus();
      let optInResult;
      if (!this.foundCmp) {
        logsConfig.errors && console.log("no CMP to opt in");
        optInResult = false;
      } else {
        logsConfig.lifecycle && console.log(`CMP ${this.foundCmp.name}: opt in on ${window.location.href}`);
        optInResult = await this.foundCmp.optIn();
        logsConfig.lifecycle && console.log(`${this.foundCmp.name}: opt in result ${optInResult}`);
      }
      if (this.shouldPrehide) {
        this.undoPrehide();
      }
      this.sendContentMessage({
        type: "optInResult",
        cmp: this.foundCmp ? this.foundCmp.name : "none",
        result: optInResult,
        scheduleSelfTest: false,
        // self-tests are only for opt-out at the moment
        url: location.href
      });
      if (optInResult && this.foundCmp && !this.foundCmp.isIntermediate) {
        this.state.endTime = Date.now();
        logsConfig.lifecycle && console.log(
          `${this.foundCmp.name}: done in ${this.state.endTime - this.state.startTime}ms with ${this.state.clicks} clicks`
        );
        this.sendContentMessage({
          type: "autoconsentDone",
          cmp: this.foundCmp.name,
          isCosmetic: this.foundCmp.isCosmetic,
          url: location.href,
          duration: this.state.endTime - this.state.startTime,
          totalClicks: this.state.clicks
        });
        this.updateState({ lifecycle: "done" });
      } else {
        this.updateState({ lifecycle: optInResult ? "optInSucceeded" : "optInFailed" });
      }
      this.restoreFocus();
      return optInResult;
    }
    async doSelfTest() {
      const logsConfig = this.config.logs;
      let selfTestResult;
      if (!this.foundCmp) {
        logsConfig.errors && console.log("no CMP to self test");
        selfTestResult = false;
      } else {
        logsConfig.lifecycle && console.log(`CMP ${this.foundCmp.name}: self-test on ${window.location.href}`);
        selfTestResult = await this.foundCmp.test();
      }
      this.sendContentMessage({
        type: "selfTestResult",
        cmp: this.foundCmp ? this.foundCmp.name : "none",
        result: selfTestResult,
        url: location.href
      });
      this.updateState({ selfTest: selfTestResult });
      return selfTestResult;
    }
    async waitForPopup(cmp, retries = 10, interval = 500) {
      const logsConfig = this.config.logs;
      logsConfig.lifecycle && console.log("checking if popup is open...", cmp.name);
      let mutationObserver = null;
      if (this.config.enablePopupMutationObserver) {
        mutationObserver = this.domActions.waitForMutation("html", 1e4);
        mutationObserver.catch(() => {
        });
      }
      const isOpen = await cmp.detectPopup().catch((e) => {
        logsConfig.errors && console.warn(`error detecting popup for ${cmp.name}`, e);
        return false;
      });
      if (!isOpen && retries > 0) {
        if (mutationObserver) {
          try {
            await Promise.all([this.domActions.wait(interval), mutationObserver]);
          } catch (e) {
            logsConfig.lifecycle && console.log(cmp.name, "popup detection timed out waiting for DOM mutation");
          }
        } else {
          await this.domActions.wait(interval);
        }
        return this.waitForPopup(cmp, retries - 1, interval);
      }
      logsConfig.lifecycle && console.log(cmp.name, `popup is ${isOpen ? "open" : "not open"}`);
      return isOpen;
    }
    prehideElements() {
      const logsConfig = this.config.logs;
      const globalHidden = [
        "#didomi-popup,.didomi-popup-container,.didomi-popup-notice,.didomi-consent-popup-preferences,#didomi-notice,.didomi-popup-backdrop,.didomi-screen-medium"
      ];
      const selectors = this.rules.filter((rule) => rule.prehideSelectors && rule.checkRunContext()).reduce((selectorList, rule) => [...selectorList || [], ...rule.prehideSelectors || []], globalHidden);
      this.updateState({ prehideOn: true });
      setTimeout(() => {
        if (this.shouldPrehide && this.state.prehideOn && !["runningOptOut", "runningOptIn"].includes(this.state.lifecycle)) {
          logsConfig.lifecycle && console.log("Process is taking too long, unhiding elements");
          this.undoPrehide();
        }
      }, this.config.prehideTimeout || 2e3);
      return this.domActions.prehide(selectors.join(","));
    }
    undoPrehide() {
      this.updateState({ prehideOn: false });
      this.domActions.undoPrehide();
    }
    updateState(change) {
      Object.assign(this.state, change);
      this.sendContentMessage({
        type: "report",
        instanceId: this.id,
        url: window.location.href,
        mainFrame: isTopFrame(),
        state: this.state
      });
    }
    async receiveMessageCallback(message) {
      const logsConfig = __privateGet(this, _config)?.logs;
      if (logsConfig?.messages) {
        console.log("received from background", message, window.location.href);
      }
      switch (message.type) {
        case "initResp":
          this.initialize(message.config, message.rules);
          break;
        case "optIn":
          await this.doOptIn();
          break;
        case "optOut":
          await this.doOptOut();
          break;
        case "selfTest":
          await this.doSelfTest();
          break;
        case "evalResp":
          resolveEval(message.id, message.result);
          break;
        case "measurePerformance":
          this.updateState({ performance: this.measurePerformance() });
          break;
      }
    }
    measurePerformance() {
      const getRoundedPerformanceEntries = (name) => performance.getEntriesByName(name).map((m) => Number(m.duration.toFixed(3)));
      return {
        detectHeuristics: getRoundedPerformanceEntries("detectHeuristics"),
        heuristicDetector: getRoundedPerformanceEntries("heuristicDetector"),
        findCmpSiteSpecific: getRoundedPerformanceEntries("findCmp_site-specific"),
        findCmpGeneric: getRoundedPerformanceEntries("findCmp_generic"),
        findCmpHeuristic: getRoundedPerformanceEntries("findCmp_heuristic"),
        parseDeclarativeRules: getRoundedPerformanceEntries("parseDeclarativeRules")
      };
    }
  };
  _config = new WeakMap();

  // rules/compact-rules.json
  var compact_rules_default = { v: 1, s: [".c-simple-popup:has(.c-cookie-popup)", ".c-cookie-popup [data-role=refuse-cookies]", ".c-cookie-popup", 'div:has(> div > a[href*="law.2gis."][href*="/privacy/cookie"])', 'div:has(> div > a[href*="law.2gis."][href*="/privacy/cookie"]) > div:last-child', "dialog.cookie-consent", "dialog.cookie-consent form.cookie-consent__form", "dialog.cookie-consent form.cookie-consent__form button[value=no]", "dialog.cookie-consent form.cookie-consent__form button.cookie-consent__options-toggle", 'dialog.cookie-consent form.cookie-consent__form button[value="save_options"]', "#abe-gdpr-banner #gdpr-banner-component", "#abe-gdpr-banner #gdpr-banner-component-decline-btn", "#gdpr-banner-component", "#gdpr-banner-component-decline-btn", "abeconsent=%221%3A1%2C2%3A0%2C3%3A0%2C4%3A0%22", 'div[id$="_cookie_box"]:has(a[onclick^="accept_"][onclick*="_cookie_box()"])', 'div[id$="_cookie_box"] a[onclick^="accept_"][onclick*="_cookie_box()"]', "#acconsento-click .acconsento-click-overlay-dialog", "#acconsento-click .acconsento-click-consent-banner", "#acconsento-click-button-necessary-only [role=button]", "#acconsento-click .acconsento-click-consent-banner .el-acconsento-dialog__headerbtn", "acconsento-clicked=1", "tracking%3D0%26marketing%3D0", '.bc-content-modal:has([class*="_cookieFooter_"])', '.bc-content-modal [class*="_cookieFooter_"]', '.bc-content-modal [class*="_cookieFooter_"] button:first-child', "%222%22%3Afalse", "div.acris-cookie-consent", "[data-acris-cookie-consent]", ".acris-cookie-consent.is--modal", "#ccAcceptOnlyFunctional", ".bg-modal:has(.preferences-modal)", ".bg-modal .preferences-modal .content button.btn-ghost.is-neutral", ".bg-modal .preferences-modal", ".preferences-modal .content button.btn-ghost.is-neutral", "cookie_consent_v2_0_marketing_third_party=0", "#__cmp_overlay", "#__cmp_body", "#__cmp_body .cmpButtonLink", "#__cmp_body .cmpLayer", "#__cmp_body .cmpLayer .vue-js-switch", "#__cmp_body .vue-js-switch.toggled .v-switch-input", "#__cmp_body .manageLink:not(.buttonDisabled)", "#__cmp_body .manageLink", "#__cmp_body .rejectLink", ".adn-window.adn-message", ".adn-window.adn-message .adn-footer button[onclick*='saveAll(false)']", "#cookie-banner", "#adopt-controller-button", "#adopt-reject-all-button", "#adroll_consent_container", "#adroll_consent_reject", "__adroll_fpc", "adstate-settings-component", ".c-cookie-notice button[data-qa='allow-all-cookies']", ".c-cookie-notice", 'button[data-qa="reject-non-essentials"]', "serif_manage_cookies_viewed", "serif_allow_analytics", ".cookie-consent-modal", ".cookie-consent-modal [data-cy=cookie-consent-accept-all-button]", ".cookie-consent-modal [data-cy=cookie-consent-reject-button]", ".cookie-consent-modal [data-cy=cookie-consent-manage-settings]", ".cookie-consent-modal [data-cy=cookie-consent-manage-settingsFR]", ".cookie-consent-modal [data-cy=cookie-consent-accept-selected-button]", ".cookie-consent-modal .cookie-selection.--full input[type=checkbox]:checked:not(:disabled)", "cookie_consent=00", "cookie_consent=10", ".alertaModal .msg.cookies", ".alertaModal .msg.cookies #rechazaCookies", "aceptaCookies=false", '.alert-bar[data-alert-api$="api/alertnotification"]:is(.alert-bar--notification, .alert-bar--cookie-message, #alert-COOKIES)', "#cookieConsent #cookiesButtomCustomize", "#cookieCustomizeWrap", "#cookieAnalyticsChk:checked", "#cookieAnalyticsChk", "#cookieConsent #cookiesButtomAccept[data-mode='custom']", "Analytics=no", ".amgdprcookie-bar-container", '.amgdprcookie-bar-container [data-amgdprcookie-js="accept"]:not([disabled])', '.amgdprcookie-bar-container [data-amgdprcookie-js="decline"]', '.amgdprcookie-bar-container [data-amgdprcookie-js="decline"]:not([disabled])', '.amgdprcookie-bar-container [data-amgdprcookie-js="settings"], .amgdprcookie-bar-container [data-amgdprcookie-js="custom_settings"]', ".amgdprcookie-settings-form", ".amgdprcookie-settings-form input.amgdprcookie-input:checked:not([disabled])", ".amgdprcookie-settings-form .amgdprcookie-done", '#am-cookie-bar [x-ref="am-cookie-bar"]', '#am-cookie-bar > [x-bind^="overlay"], #am-cookie-bar [data-dialog-ref-name="am-cookie-bar"]', '#am-cookie-bar[style*="translateY(100%)"]', '[x-ref="am-cookie-bar-settings"] :is([x-ref="ambar-settings-done"], button[\\@click*="cookiebar-action-accept"])', '#am-cookie-bar [x-ref="am-cookie-bar"] :is([data-dispatch-action="cookiebar-action-accept"], .ambar-btn-decline)', '#am-cookie-bar [x-ref="am-cookie-bar"] :is([data-dispatch-action="cookiebar-action-settings"], .ambar-btn-settings)', '[x-ref="am-cookie-bar-settings"] input[type="checkbox"]', '[x-ref="am-cookie-bar-settings"] input[type="checkbox"]:checked:not([disabled])', '#am-cookie-bar [x-ref="am-cookie-bar"] :is(button[\\@click="closeCookieBarHandler"], button[\\@click*="close-cookie-bar"])', '[data-test-id="consentModal"] [data-test-id="consentBtn"]', '[data-test-id="consentModal"] button[aria-label="Continue without accepting cookies"]', '[data-test-id="consentModal"] button[aria-label="Close cookie consent card popup"]', '[data-test-id="consentModal"]', '#sp-cc-wrapper,[data-action=sp-cc],span[data-action="sp-cc"][data-sp-cc*="rejectAllAction"]', '#sp-cc-wrapper *,[data-action=sp-cc],span[data-action="sp-cc"][data-sp-cc*="rejectAllAction"]', "#sp-cc-rejectall-link", "#user-consent-management-granular-banner-overlay", "[data-testid=granular-banner-button-decline-all]", "div:has(> p > a[href='https://www.anthropic.com/legal/cookies'])", "div:has(> p > a[href='https://www.anthropic.com/legal/cookies']) button:nth-child(2)", "anthropic-consent-preferences", "iframe[srcdoc*='cdn.appconsent.io/tcf2-clear/']", "script[src*='cdn.appconsent.io/tcf2-clear/'][src*='core.bundle.js']", "iframe[srcdoc*='frame-root']", "script[src*='cdn.appconsent.io/tcf2/'][src*='core.bundle.js']", "#consent-tracking", "#consent-tracking .decline.btn", ".modal-open bahf-cookie-disclaimer-dpl3", "bahf-cookie-disclaimer-dpl3", "bahf-cookie-disclaimer-dpl3:not([aria-hidden=true])", "cookie_consent=denied", ".Dialog:has(.ConsentManager)", ".ConsentManager[data-controller~='consent-manager']", ".ConsentManager button[data-action*='consent-manager#denyAll']", "arto-consent=signature", 'dialog[data-testid="cookie-message-modal"]', '[data-testid="cookie-message-bottom-sheet"]', '[data-testid="cookie-message-bottom-sheet-overlay"]', 'dialog[data-testid="cookie-message-modal"] button[data-testid="decline-all-cookies"], [data-testid="cookie-message-bottom-sheet"] button[data-testid="decline-all-cookies"]', 'dialog[data-testid="cookie-message-modal"], [data-testid="cookie-message-bottom-sheet"]', "#gdpr_area", "#gdpr_area .gdpr_area_inner", "#gdpr_area .gdpr_btn_agree, #gdpr_area #gdpr_btn_agree", "is_gdpr_agree=1", "#cookie_consent", "#cookie_consent_manage_rec", "#cookie_consent_manager_confirm", ".cookie_consent_manager_accept_req", "#cookie_consent_manage", "#cookie_consent_manager input[type=checkbox]:checked:not(:disabled)", "air_cookie_consent=", "false}", "#divCookieInfo", "#divCookieInfo #ctl00_CookieInfo_btnLinkNO", " - OFF - ", 'div[role="region"][aria-label="Cookie notice"][data-astro-cid-fzbxxtek]', 'div[role="region"][aria-label="Cookie notice"][data-astro-cid-fzbxxtek] > button[data-astro-cid-fzbxxtek]', "#cookie-policy-info,#cookie-policy-info-bg", "#cookie-policy-info button", "#cookie-policy-info .btn-reject", "#cookie-policy-info .btn-setting", "#cookie-policy-info .btn-ok", "#RodoCookiesInfoFirst", "#RodoCookiesInit", '#RodoCookiesInfoFirst [data-type="disable-cookies-button"], #RodoCookiesInit [data-type="disable-cookies-button"], #RodoCookiesInfo [data-type="disable-cookies-button"]', "#RodoCookiesInfoFirst.in, #RodoCookiesInit.in, #RodoCookiesInfo.in", "#RodoCookiesInfoFirst.in", '#RodoCookiesInfoFirst [data-type="disable-cookies-button"]', "#RodoCookiesInit.in", '#RodoCookiesInit [data-type="disable-cookies-button"]', '#RodoCookiesInfo [data-type="disable-cookies-button"]', "AtomStore[MARKETING_COOKIES_ACCEPTED]=0", "div[data-popup-cookies][data-terms-cookies-popup-common]", 'div[data-popup-cookies][data-terms-cookies-popup-common] button[data-cookies="disallow_all_cookies"]', "all_marketing_cookies=denied", 'form[class*="a8c-cookie-banner"][method="post"]', 'form[class*="a8c-cookie-banner"] div[class*="simple-options"] a[class*="customize-button"]', "input[type=checkbox][checked]:not([disabled])", 'a[class*="accept-selection-button"]', "#as24-cmp-popup", '#as24-cmp-popup [data-testid="as24-cmp-container"]', '#as24-cmp-popup [data-testid="as24-cmp-decline-all-button"]', "%22consents%22%3A%5B%5D%7D%2C%22vendor%22", ".as-oil", ".AvacyBanner", ".as-oil, .AvacyBanner", ".as-oil .as-js-reject-all, .AvacyBanner .as-js-reject-all", ".as-oil .as-js-only-essentials", ".as-oil .as-js-close-banner, .AvacyBanner .as-js-close-banner", ".as-oil .as-js-advanced-settings", "#as-oil-cpc .as-js-optin", "#as-oil-cpc .as-js-btn-deactivate-all", ".Avada-CookiesBar__OuterWrapper .Avada-CookiesBar_Container", ".Avada-CookiesBar__OuterWrapper .Avada-CookiesBar_BtnDeny", ".Avada-CookiesBar-Preferences__PopUp .Avada-CookiesBar_BtnDeny", '"analytics":false,"marketing":false', ".fusion-privacy-bar", '.fusion-privacy-bar .fusion-privacy-bar-reject, .fusion-privacy-bar .fusion-privacy-choices input[type="checkbox"]:not(:disabled)', ".fusion-privacy-bar .fusion-privacy-bar-reject", '.fusion-privacy-bar .fusion-privacy-choices input[type="checkbox"]:checked:not(:disabled)', ".fusion-privacy-bar .fusion-privacy-bar-acceptance", "#avg-cookiebanner", "__avgAskUser=false", "#avg-cookiebanner:not(.state-close)", "#avg-cookiebanner .rejectAll", "#avg-cookiebanner .avg-cookie:checked", "#avg-cookiebanner .banner-panel .close", "#avg-cookiebanner .save", "#ACClayerBackground", "#ACClayerBackground #ACCsmallCookieLayer", "#ACCsmallCookieLayer, #ACCtabedCookieLayer", "#ACCsmallCookieLayer", "#ACCsmallCookieLayer .kkACCButtonLearnMore", "#ACCdetailsSaveButton", "#ACCtabedCookieLayer .kkACCSwitchLabel input:checked", "#ACCtabedCookieLayer .kkACCSwitchLabel:has(input:checked)", "=1000,", "#awsccc-cb-content", "#awsccc-cs-container", "#awsccc-cs-modalOverlay", "#awsccc-cs-container-inner", "button[data-id=awsccc-cb-btn-decline]", "button[data-id=awsccc-cb-btn-customize]", "input[aria-checked]", "input[aria-checked=true]", "button[data-id=awsccc-cs-btn-save]", ".axeptio_widget,.axeptio_mount", ".axeptio-widget--open", ".axeptio_mount .axeptio_widget", "%22$$completed%22:false", ".axeptio_mount .needsclick", "button#axeptio_btn_dismiss,button.ax-discardButton,button#axeptio_btn_close", "button#axeptio_btn_dismiss,button.ax-discardButton", "button#axeptio_btn_close", "axeptio_authorized_vendors=%2C%2C", "body > [style*='--ax-primary-color']", "script[src*='axept.io/tcf/']", "#cookie_consent_wrapper, #cookie_consent_min_wrapper, #manage_cookie_consent_modal", "#cookie_consent_wrapper:not(.hideConsent)", "#cookie_consent_wrapper #consent_accept_essential, #manage_cookie_consent_modal #modal_consent_accept_essential", "#cookie_consent_wrapper.non_cookie_consent_wrapper #js_cookieBannerCloseBtn", "#cookie_consent_wrapper #consent_accept_essential", "#cookie_consent_wrapper #js_cookieBannerCloseBtn", "#manage_cookie_consent_modal #modal_consent_accept_essential", "#cookie_consent_min_wrapper #close_minimized_banner", "cookieConsent=1", "cookieConsent=5", ".b-cookie.is-open", ".cookie-alert.t-dark", "#cookie-accept-necessary", ".cookie-alert .checkbox__input:checked:not(:disabled)", ".cookie-alert__button button", "#banner-privacy", "#banner-privacy #cb-RejectAll", "cookiepolicy=1-", '[id$="bathnes-cookie-consent-banner"] #cookie-banner', '[id$="bathnes-cookie-consent-banner"] #cookie-banner button[onclick^="rejectCookies"]', "#cookie.modal", "#cookie.modal #btn_cookie_submit", "#cookie.modal #cookie_stat:checked", "#cookie.modal #cookie_stat", "iabbb-cookies-message > section", "iabbb-cookies-message", "iabbb-cookies-message > section [name='allow-all']", "iabbb-cookies-message > section button.bds-button-unstyled", "iabbb-cookies-message dialog[open]", "iabbb-cookies-message dialog fieldset input[type=radio][value=false]:not(:checked)", "iabbb-cookies-message dialog button.bds-button", "iabbb-cookies-message > section button.bds-button:not([name])", ".cookiesgdpr__base:has(.cookiesgdpr__rejectbtn)", ".cookiesgdpr__base .cookiesgdpr__rejectbtn", ".cookiesgdpr__base .cookiesgdpr__wrapper", '"analitica":false', "#overlay-policy", "#overlay-policy #policy_pop_close .fa-times-circle", "#overlay-policy #policy_pop_close", "showpopuppolicy=No", "beetle-tracking-consent-modal", '"bt_marketing":false', "#mfn-consent-mode", "#mfn-consent-mode #consent_deny", "body.mfn-cookies-open #mfn-consent-mode .mfn-cookies-popup", "%22analytics_storage%22%3A%22denied%22", "#consent-manager", "#consent-manager button", "#consent-manager button:nth-child(2)", "tracking-preferences", "#bc-cmp-host", "#bc-cmp-host-auto", "#bigidcmp-banner-widget", "bigidcmp-consent-", "marketing=false", "#bnp_container", "#bnp_cookie_banner", "#bnp_btn_accept,#bnp_btn_reject", "#bnp_btn_reject", "AD=0", "#binogi_cookie_notice", "#binogi_cookie_notice #cnreject", "#cconsent-modal", "#cconsent-modal .ccm__footer .consent-givenone", "#cconsent-modal.ccm--visible .ccm__footer .consent-givenone", '"wanted":false', ".cookie-notification", "#blocksy-ext-cookies-consent-styles-css", ".cookie-notification .ct-cookies-decline-button", "blocksy_cookies_consent_accepted=no", "#cookieChoiceInfo.cookie-choices-info", "#cookieChoiceInfo.cookie-choices-info #cookieChoiceDismiss", "#cookieChoiceInfo #cookieChoiceDismiss", "displayCookieNotice=y", "#cookie_banner:has(#cookie_essential)", "#cookie_banner #cookie_buttons #cookie_essential", "#cookie_banner #cookie_essential", "#cookie_banner", "#CookieConsentContainer:has(#BtnCCPrefs) > .container", "#CookieConsentContainer #BtnCCPrefs", "#CookieConsentContainer #BtnCCAll", "#CookieConsentContainer #BtnCCNone", "#CookieConsentPanel #BtnCCSave", "#ConsentAnalytics:checked", "#ConsentAnalytics", "#ConsentThirdParty:checked", "#ConsentThirdParty", "-saved=true", "-analytics=true", "#cookie-consent-banner", "#ccpa-consent-banner", '#cookie-consent-banner [data-id="cookie-consent-banner-modal"]', "#cookie-consent-banner .banner-gdpr, #cookie-consent-banner #gdpr-modal", '#ccpa-consent-banner [data-id="btn-close-ccpa"]', '#cookie-consent-banner [data-id="btnGdprRejectAll"], #cookie-consent-banner [data-id="btn-contd-without-accpting"]', '#cookie-consent-banner [data-id="pre-banner-btn"]', '#cookie-consent-banner [data-id="cookie-consent-banner-modal"] input[type="checkbox"]:checked:not(:disabled)', '#cookie-consent-banner [data-id="btn-save-preference"]', "#BorlabsCookieBox", "._brlbs-block-content,.brlbs-cmpnt-dialog", "._brlbs-bar-wrap,._brlbs-box-wrap,.brlbs-cmpnt-dialog", '.brlbs-btn-accept-only-essential,a[data-cookie-refuse],.brlbs-btn-save,#CookieBoxSaveButton,.brlbs-cmpnt-close-button[data-borlabs-cookie-actions="close-button"],a[data-cookie-individual]', ".brlbs-btn-accept-only-essential,a[data-cookie-refuse]", ".brlbs-btn-save,#CookieBoxSaveButton", '.brlbs-cmpnt-close-button[data-borlabs-cookie-actions="close-button"]', "a[data-cookie-individual]", ".cookie-preference", "input[data-borlabs-cookie-checkbox]:checked", "#CookiePrefSave", "dock-privacy-settings", ".js-cookie-consent:has(.js-cookie-consent-reject)", ".js-cookie-consent .js-cookie-consent-reject", ".js-cookie-consent:not(.is-hidden) .js-cookie-consent-reject", "#cookie-panel-banner", "#cookie-panel-banner-reject-button", "no.bouvet.app.cookiepanel-cookie-control", "#cconsent-bar", "#cconsent-bar .consent-give", "#cconsent-bar:not(.ccb--hidden) .consent-give", "#cconsent-bar button.consent-decline", "#cconsent-bar .ccb__edit", "#ccm__footer__consent-modal-submit", '#cconsent-modal .ccm__switch-group[aria-checked="true"]', "#cconsent-modal input.category-onoff:checked:not(:disabled)", "#bs-gdpr-cookies-modal", "#bs-gdpr-cookies-modal #bs-gdpr-cookies-modal-advanced-btn", "#bs-gdpr-cookies-modal #bs-gdpr-cookies-modal-accept-btn", "#bs-gdpr-cookies-modal-advanced-types", "#bs-gdpr-cookies-modal-advanced-types input[type=checkbox]:checked:not(:disabled)", "CookieShow=true", "#bsw-consentCookie", "#bsw-consentCookie .accept-btn", "#bsw-consentCookie .manage-cookies-btn", "#bsw-consentCookie .config-banner_section", "#bsw-consentCookie input[type=checkbox]:checked:not(:disabled)", "x-bsw-consentCookie", ".cookie-banner #accept_all", "#cookies_modal #third_party_cookies_toggle", "#third_party_cookies_toggle:checked", "#third_party_cookies_toggle", "#cookies_modal #save_button", ".cookie-banner:not(.d-none) #accept_all", "third_party_cookies_accepted=False", "cookie_banner_hidden=True", ".cookiebanner--tablist", "#cookiebanner[data-cookiebanner]", '.cookiebanner [data-consent="acceptSelection"]', '.cookiebanner input[type="checkbox"]:checked:not(:disabled)', '.cookiebanner input[type="checkbox"]:checked:not(:disabled) + label', ".bpa-cookie-banner", ".bpa-cookie-banner .bpa-module-full-hero", ".bpa-close-button", "cookie-allow-tracking=0", ".cookie-modal", "#notice-cookie-block", "#notice-cookie-block #cm-selectSpecific", "#html-body #notice-cookie-block", "#notice-cookie-block #btn-cookie-decline", "#notice-cookie-block button[\\@click^='rejectAll']", "#btn-cookie-manage", "#notice-cookie-block input:checked", "#btn-cookie-save", "#html-body #notice-cookie-block, #notice-cookie", ".bwr-cookie.js-cookie", ".bwr-cookie.js-cookie .js-cookie-trigger", ".bwr-cookie.js-cookie .bwr-cookie__buttons", ".bwr-cookie.js-cookie input[data-cookie-option]:checked:not(:disabled)", "ZnVuY3Rpb25hbCI6ZmFsc2UsInBlcnNvbmFsaXphdGlvbiI6ZmFsc2V9", "#cookieconsent", "#cookieconsent .cookiechoices .rejectcookies", "ANALYTICALCONSENT=0", "MARKETINGCONSENT=0", "#cookieBanner.c-banner", "#cookieBanner.c-banner button[data-allow-cookies=false]", "cookies_allow=false", "div.c-cookie-message.fixed", "div.c-cookie-message.fixed > div > button", "[data-testid='cookie-banner-root'][class*='c15t-'], [data-testid='consent-banner-root'][class*='c15t-'], [data-testid='cookie-banner-overlay'][class*='c15t-'], [data-testid='consent-banner-overlay'][class*='c15t-']", "[data-testid='cookie-banner-root'][class*='c15t-'], [data-testid='consent-banner-root'][class*='c15t-']", "[data-testid='cookie-banner-reject-button'], [data-testid='consent-banner-reject-button']", "[data-testid='cookie-banner-customize-button'], [data-testid='consent-banner-customize-button']", "[data-testid='consent-widget-footer-save-button']", "[data-testid^='consent-widget-switch-'] input[type='checkbox']:checked:not(:disabled)", "c.necessary:1", "c15t=v=", "c.marketing:1", "mk=1.", "#cookiesLayer", "#cookiesLayerInn .act-cook[data-cook='2']", "#cookiesLayerInn", "#cookiesLayerConfig #btn_no_cookies", "conf_cookies=2%3A2%3A2%3A2", ".js-cookies-policy-alert", '.js-cookies-policy-alert [class*="cookies-policy-alert-body"] [onclick*="js-cookies-policy-alert"]', '.js-cookies-policy-alert [class*="cookies-policy-alert-body"]', ".cassie-cookie-module", ".cassie-pre-banner", "#cassie_pre_banner_text", ".cassie-reject-all", ".cc_banner-wrapper", ".cc_banner", ".cc-banner[data-cc-banner]", ".cc-banner[data-cc-banner] button[data-cc-action=reject]", ".cc-banner[data-cc-banner] button[data-cc-action=preferences]", ".cc-preferences[data-cc-preferences]", ".cc-preferences[data-cc-preferences] input[type=radio][data-cc-action=toggle-category][value=off]", ".cc-preferences[data-cc-preferences] button[data-cc-action=reject]", ".cc-preferences[data-cc-preferences] button[data-cc-action=save]", "#ccm-widget.ccm-show", "#ccm-widget", "script[src*='ccm19'][src*='/app.js?apiKey=']", "#ccm-widget .ccm--decline-cookies", "#ccm-widget [data-ccm-modal='ccm-control-panel']", "#ccm-control-panel .ccm--save-settings:not([data-full-consent='true'])", ".ccoo-cc-root", ".ccoo-cc-root #sn-b-save", ".ccoo-cc-root .ccoo-cc-dialog", ".ccoo-cc-root #sn-b-custom", ".ccoo-cc-root #adv-cookies-3", ".ccoo-cc-root #adv-cookies-3 input[type=checkbox]:checked", "config=0", "#js_cookie-consent-banner", "#js_cookie-consent-banner .js_cookie-consent-necessary", ".cfGdprCookieConsentPopup", ".cfGdprCookieConsentPopup #cfGdprCookieConsentPopupSettings", ".cfGdprCookieConsentPopup #cfGdprCookieConsentPopupSaveSettings", ".cfGdprCookieConsentPopup__settings input[type=checkbox]:checked", "CF_GDPR_COOKIE_CONSENT_PERFORMANCE=0", "CF_GDPR_COOKIE_CONSENT_SOCIAL=0", "CF_GDPR_COOKIE_CONSENT_MARKETING=0", ".c24-cookie-consent-wrapper", ".c24-cookie-consent-wrapper .c24-cookie-consent-functional", ".c24-cookie-consent-notice", "[data-modal-content]:has([data-toggle-target^='cookie'])", "[data-toggle-target^='cookie']", "[data-cookie-dismiss-all]", ".chefcookie", ".chefcookie .chefcookie__buttons", ".chefcookie .chefcookie__box", '.chefcookie .chefcookie__buttons a[href="#chefcookie__decline"]', "hide_prompt=1", '.gdpr-modal:has(img[src*=".ciaopeople.it/"])', '.gdpr-modal .gdpr-modal__box-top-sx img[src*=".ciaopeople.it/"]', ".gdpr-modal .gdpr-modal__box", ".gdpr-modal__box-top-dx > button", 'section[role="alert"]:has(button[aria-label="close cookie message"])', 'section[role="alert"] button[aria-label="close cookie message"]', "#ccc-module,#ccc-overlay,#ccc", "#ccc-module,#ccc-notify", "#ccc", ".ccc-reject-button", "#ccc-notify .ccc-notify-buttons .ccc-notify-button:not(.ccc-accept-button):not(.ccc-reject-button)", "#ccc-module", "#ccc-dismiss-button", "#ccc-close", ".cc-individual-cookie-settings", ".cc-individual-cookie-settings #cookie-settings-reject-all", "ckies_cookielaw", "#cksModal", "#cksModal #cks .cksButtons", "#cksModal #cks", `#cks .cksLink[onclick*="setCksPrefs('0')"]`, "CookiePrefs=0", "#cl-consent", '#cl-consent [data-role="b_options"]', '.cl-consent-popup.cl-consent-visible [data-role="alloff"]', '[data-role="b_save"]', "__lxG__consent__v2_daisybit=", ".consent-modal[role=dialog]", "#consent_reject", "#manage_cookie_preferences", "#cookie_consent_preferences input:checked", "#consent_save", "ctc_rejected=1", "#cookie-notice:has(.cookie-notice-close[onclick^='closeCookieNotice'])", "#cookie-notice .cookie-notice-close[onclick^='closeCookieNotice']", ".cf_modal_container", "#cm-wrapper:has(#cm-selectSpecific)", "#cookie-consent:has(#cm-selectSpecific)", "#cm-selectSpecific", "#cm-selectAllCheckboxes", "#cm-openSettings", ".cmCat-checkboxes:checked:not(:disabled)", 'app-cookie-consent[data-component="site-base"]', 'app-cookie-consent[data-component="site-base"] .cookie-buttons .cookie-refuse', 'app-cookie-consent[data-component="site-base"] .cookie-buttons', 'app-cookie-consent[data-component="site-base"] .cookie-refuse', "#cookiesConsentBanner", "#cookiesParametersModal", "#cookiesConsentBanner .refuse-all-cookies", "#cookiesParametersModal .refuse-all-cookies", "cookiesConsentDone=true", "%22performanceCookies%22:false", "#coki61 .cok_but1", "#coki61", "coki61=2", "#cu-privacy-notice", "#cu-privacy-notice .cu-privacy-notice-close", "com-consent-layer", "#cmplz-cookiebanner-container", "#cmplz-cookiebanner-container .cmplz-cookiebanner", ".cmplz-cookiebanner .cmplz-deny", "cmplz_banner-status=dismissed", '.cc-type-categories[aria-describedby="cookieconsent:desc"]', '.cc-type-categories[aria-describedby="cookieconsent:desc"] .cc-dismiss', ".cc-dismiss", ".cc-type-categories input[type=checkbox]:not([disabled]):checked", ".cc-save", '.cc-type-info[aria-describedby="cookieconsent:desc"]', '.cc-type-info[aria-describedby="cookieconsent:desc"]:not(.cc-invisible) .cc-compliance .cc-btn', '.cc-type-info[aria-describedby="cookieconsent:desc"] .cc-compliance .cc-btn', ".cc-deny", '[aria-describedby="cookieconsent:desc"]', ".cc-blockingScreen", '[aria-describedby="cookieconsent:desc"].cc-type-opt-out', '[aria-describedby="cookieconsent:desc"].cc-type-opt-out:not(.cc-invisible)', ".cmp-pref-link", ".cmp-body .cmp-save-btn", ".cmp-body [id*=rejectAll]", '.cc-type-opt-in[aria-describedby="cookieconsent:desc"]', '.cc-type-opt-in[aria-describedby="cookieconsent:desc"]:not(.cc-invisible)', ".cc-settings", ".cc-settings-view", ".cc-settings-view input[type=checkbox]:not([disabled]):checked", ".cc-settings-view .cc-btn-accept-selected", '.cc-type-opt-in[aria-describedby="cookieconsent:desc"].cc-invisible', "#cookie-banner.cookie-banner", "#cookie-consent", "#cookie-button-settings-accept", "#cookie_consent_Necessary", "#cookie-button-settings", "#cookie_consent_Product:checked, #cookie_consent_Marketing:checked", "#cookieDisclaimer", "#cookieDisclaimer #_com_comundus_cookies_CookieBanner_cookieConsentForm", "#cookieDisclaimer #showCookieSettingsBtn", "#_com_comundus_cookies_CookieBanner_updateCookieConsentBtn", "#cookieDisclaimer input.banner-group-consent:not(.disabled):checked + label", ".cookie-consent-card:has([data-cy=cookie-customize])", ".cookie-consent-card [data-cy=cookie-customize]", "#cookie-preferences-dialog [data-cy=cookie-save-preferences]", "#cookie-preferences-dialog button[role=switch][aria-checked=true]:not([aria-disabled=true])", "_cookieConsent=%7B%22analytics%22%3Afalse%2C%22marketing%22%3Afalse", ".cookiebanner:has(> #cookieform .chooseCookies)", ".diffusion_layer:has(#cookieform)", ".cookiebanner #cookieform .chooseCookies", ".cookiebanner [onclick^='sendForm(0']", ".cookiebanner .chooseCookies input[type='checkbox']:checked:not([onclick])", ".cookiebanner [onclick^='sendForm(1']", ".consent-backdrop > .consent[role=dialog][aria-labelledby=consent-title]", ".consent-backdrop > .consent[role=dialog][aria-labelledby=consent-title] .consent__actions .consent__action--secondary", ".consent-backdrop .consent__groups input[type=checkbox]:checked:not(:disabled)", ".consent-backdrop .consent__actions .consent__action--secondary", "consent_", "._flo-consent-root", "._flo-consent-root ._flo-consent-modal-contextual ._flo-consent-button--accept-selected", "._flo-consent-root ._flo-consent-modal:not(._flo-consent-modal-contextual)", "._flo-consent-modal:not(._flo-consent-modal-contextual) ._flo-consent-button--reject", "._flo-consent-button--settings", "._flo-consent-modal-contextual ._flo-consent-button--accept-selected", "._flo-consent-modal-contextual ._flo-consent-checkbox:checked:not(._flo-consent-checkbox--necessary)", "._flo-consent-root._flo-consent-hide", ".consent-studio .cookie-banner", ".consent-studio .consent-banner", ".consent-studio [cs-role=consent-banner]", ".consent-studio [cs-role=consent-banner] [data-cs-button=reject_all]", ".consent-studio [cs-role=consent-banner] [cs-consent=reject]", ".consent-studio [cs-role=consent-banner] .cookie-banner__switch[aria-checked=true]:not([aria-disabled=true])", ".consent-studio [cs-role=consent-banner] [data-cs-button=save]", "consent-studio__storage=[%22functional%22]", "#consentaur-banner", "#consentaur-banner .consentaur-btn-reject", "%22marketing%22%3Afalse", '.cs-info-bar[class*="cs-public-cookie-"] .cs_action_btn[data-cs_action="allow_all"]', '.cs-info-bar .cs_action_btn[data-cs_action="disable_all"]', ".cs-info-bar .options_btn", '.cs_settings_popup [data-cs_action="cs_confirm"]', ".cs_settings_popup input.cs_script_action:checked:not(:disabled)", "cs_viewed_cookie_policy=", "#ncmp__tool .ncmp__banner", "#ncmp__tool .ncmp__banner.ncmp__active", "ncmp=", "csm-cookie-consent", "#cookie-information-template-wrapper", "#CcpaWrapper", "CookieInformationConsent=", ".cookie.cookie--open", ".cookie .cookie__settings .cookie__save-settings", ".cookie .cookie__open-settings", ".cookie.cookie--open .cookie__open-settings", ".cookie .cookie__settings .cookie__decline-all", ".cookie .cookie__settings input.switch-toggler__checkbox:checked:not(:disabled)", ".cookie-consent:has(> .cookie-text + .cookie-agree:last-child)", ".cookie-consent > .cookie-text + .cookie-agree:last-child", "cookie-banner-element#ckb", "#cookie-bar", "#cookie-bar-prompt", "#cookie-bar > #cookie-bar-button", "#cookie-bar-prompt #cookiebar-save-customized", "#cookie-bar-button-customize", "#cookiebar-save-customized", "#cookiebar-tracking:checked", "#cookiebar-tracking", "#cookiebar-third-party:checked", "#cookiebar-third-party", "cookiebar=CookieCustomized", "cookiebar-tracking=false", "#cookie-bar .cookie-bar-frame #decline", "#cookie-bar .cookie-bar-frame", "cookies-state=decline", "xpath///div[contains(@class,'cookie-box__in')]//*[contains(concat(' ',normalize-space(@class),' '),' cookie-box__btn ')][normalize-space()='OK' or normalize-space()='\uFF2F\uFF2B' or normalize-space()='\u78BA\u8A8D\u3057\u307E\u3057\u305F' or normalize-space()='\u9589\u3058\u308B']", ".cookie-box__in.js-load--on:not(.js-load)", "[data-cookie-consent-root] [data-cookie-consent-banner]", "[data-cookie-consent-root] [data-cookie-consent-banner] [data-cookie-consent-open-manage]", "[data-cookie-consent-root] [data-cookie-consent-open-manage]", "[data-cookie-consent-root] [data-cookie-consent-save]", "[data-cookie-consent-root] input[data-cookie-consent-analytics]:checked", "[data-cookie-consent-root] input[data-cookie-consent-analytics]", "[data-cookie-consent-root] input[data-cookie-consent-marketing]:checked", "[data-cookie-consent-root] input[data-cookie-consent-marketing]", "%22analytics%22%3A0%2C%22marketing%22%3A0", ".spicy-consent-wrapper", ".spicy-consent-bar", ".js-decline-all-cookies", "section.cookie-info", "section.cookie-info .js-cookie-info__decline", "section.cookie-info.is-visible:not(.is-hidden)", "accepted-cookies=functional", "accepted-cookies=functional%2C", "#cookie-law-info-bar, #cookie-law-bg, .cli-popupbar-overlay", "#cookie-law-info-bar", "cookielawinfo-checkbox-non-necessary=yes", "#btn-cookie-settings", "#notice-cookie-block #allow-functional-cookies, #notice-cookie-block #btn-cookie-settings", "#allow-functional-cookies", ".modal-body", '.modal-body input:checked, .switch[data-switch="on"]', '[role="dialog"] .modal-footer button', "#cookie-notice:has(#cookie-notice-accept.cookie-notice__accept--custom)", "#cookie-notice #cookie-notice-accept.cookie-notice__accept--custom", "#cookie-notice #cookie-notice-reject", "#cookie-notice input[type=checkbox]:not(#cookie-checkbox-mandatory, [data-is-mandatory], :disabled):checked", "#cookie-notice #cookie-notice-accept", "#cookie-popup", "#cookie-popup #close-cookies", "#cookie-popup #view-all", '#cookie-popup:not([class*="invisible"]) #close-cookies', '#cookie-popup[class*="invisible"]', "acceptedAnalytics=granted", '#cookie-popup:has([onclick*="cookieNo()"])', '#cookie-popup [onclick*="cookieNo()"]', '#cookie-popup:has([onclick*="cookieOk()"]):not(:has([onclick*="cookieNo()"]))', '#cookie-popup:not(:has([onclick*="cookieNo()"])) [onclick*="cookieOk()"]', "form.cookie-pref-form .cookie-bar", "form.cookie-pref-form .cookie-bar .cookie-bar--action", "form.cookie-pref-form .cookie-bar .cookie-bar--reject", "form.cookie-pref-form .show-cookie-pref", "form.cookie-pref-form .cookie-pref-save", "form.cookie-pref-form .cookie-pref-overlay input[type=checkbox]:checked:not(:disabled)", "#cookiescript_injected", ".cookiescript_banner_container [role=dialog]", "#cookiescript_injected, .cookiescript_banner_container", "#cookiescript_injected, .cookiescript_banner_container [role=dialog]", "#cookiescript_reject", "#cookiescript_manage", ".cookiescript_fsd_main, #cookiescript_save", ".cookiescript_fsd_main", "#cookiescript_save", "#cookiescript_injected input.cookiescript_checkbox_input:checked:not(:disabled), .cookiescript_banner_container input.cokiescript_checkbox:checked:not(:disabled)", "#cookiescript_close, .cookiescript_close_btn", ".cookie-use-notice", ".cookie-use-notice #decline-cookie", "-cookie=deny", "#cookieAcceptBar.cookieAcceptBar", "#cbPopupWrap.cb-popup--open", "#cbPopupWrap #cbRefuseTrigger", "cbDataAccepted=1", "cookiecode-banner", 'script[src*="cdn.cookiecode.nl/dist/"]', "#cc--main", "html.show--consent #cm, html.show--settings #s-cnt", "#s-all-bn", "#s-rall-bn", "#c-s-bn", "#s-bl input.c-tgl:checked:not(:disabled)", "#s-sv-bn", "cc_cookie=", "#cc_div", "#cc_div #cm_btns", "html.show--consent #cc_div #cm, html.lb-show-cookie-consent #cc_div #cm", "#cm_secondary_btn", "html.show--settings, html.lb-show-cookie-settings", "#cs_blocks input.c_toggle:checked:not(:disabled)", "#cs_save__btn", "lb_csc=", "#cc-main", "#cc-main .cm-wrapper", ".cm__btn[data-role=necessary]", ".cm__btn[data-role=show]", "#cc-main .pm__btn[data-role=necessary]", ".cc-cookies", ".cc-cookies .cc-cookie-accept", ".cc-cookies:not(#cookies_declinedbar) .cc-cookie-accept", ".cc-cookies .cc-cookie-decline", "#cookiefirst-root,.cookiefirst-root,[aria-labelledby=cookie-preference-panel-title]", "#cookiefirst-root,.cookiefirst-root", "#cookiefirst-root button[data-cookiefirst-action],.cookiefirst-root button[data-cookiefirst-action],#btnAcceptAllCookies,#btnRejectAllCookies,#btnAdjustSettings,[aria-labelledby=cookie-preference-panel-title] button", "button[data-cookiefirst-action=reject]", "#btnRejectAllCookies", "button[data-cookiefirst-action=adjust]", "[data-cookiefirst-widget=modal]", "button[data-cookiefirst-action=save]", ".ch2-container", ".ch2-dialog", ".ch2-open-settings-btn, .ch2-open-personal-data-btn", ".ch2-settings", ".ch2-deny-all-btn", ".ch2-settings input[type=checkbox]:not([disabled]):checked", ".ch2-save-settings-btn", "cookiehub=", ".cookieinfo", ".cookieinfo .cookieinfo-close", ".cookieinfo-close", ".JSWrapper:has(#cookieInfoZamknij)", "#cookieInfoZamknij", "js_on=1", ".cookiejs-banner-wrapper", ".cookiejs-banner-wrapper:not(.modal)", ".cookiejs-banner-wrapper button.cookiejs-button", ".cookiejs-banner-wrapper #reject-cookies-btn", "analytics_cookies=false", ".cookiejs-banner-wrapper.modal", ".cookiejs-banner-wrapper .cookiejs-banner-text", ".cookiejs-banner-wrapper .cookiejs-banner-li:nth-of-type(2) button", "cookiejs_preferences", "#cookie-legit-notice-container .cookie-legit-notice", "#cookie-legit-notice-container.notice .cookie-legit-notice", "#cookie-legit-notice-container .cookie-legit-deny-btn", "#cookie-legit-notice-container .cookie-legit-pref-btn", "#cookie-legit-notice-container .cookie-legit-save-pref-btn", "#cookie-legit-notice-container input[name=cl_tracking]:checked", "#cookie-legit-notice-container input[name=cl_marketing]:checked", "cl_tracking=false", "cl_marketing=false", "#cookieman-modal", "#cookieman-modal [data-cookieman-save]", "#cookieman-modal [data-cookieman-accept-none]", '#cookieman-modal [aria-controls="cookieman-settings"]', "#cookieman-modal [data-cookieman-form]", '#cookieman-modal [data-cookieman-form] input[type="checkbox"]:checked:not(:disabled)', "#cookieman-modal [data-cookieman-save]:not([data-cookieman-accept-all]):not([data-cookieman-accept-none])", "CookieConsent=", "#cp-overlay", "#cp-overlay #cp-no a", "cookiepermission=no", "#eksCookiePrompt", "#eksCookiePrompt .cpDontAcceptBtn", "#eksCookieNo", "cookieOptOut=n", "#CookieReportsBanner", "#CookieReportsOverlay", "#CookieReportsPanel #CookieReportsBanner", ".modal-cacsp-position", ".modal-cacsp-backdrop", ".modal-cacsp-position .modal-cacsp-box-info", ".modal-cacsp-box-info.modal-cacsp-box-show", ".modal-cacsp-box-info .modal-cacsp-btn-refuse", ".modal-cacsp-box-info .modal-cacsp-btn-settings", ".modal-cacsp-box-settings.modal-cacsp-box-show .modal-cacsp-btn-save", "cookies_and_content_security_policy=[]", ".cookies-tip-v1-consent-container", ".cookies-tip-v1-consent-container [data-cky-tag=close-button]", "#cookiesBanner:has(#cookiesBannerPreference)", "#cookiesBanner #cookiesBannerPreference", "#cookiesBanner:not(.cookiesBannerHide) #cookiesBannerBox", "#cookiesBannerInfos button[onclick^='cookieSettingsShow']", "#cookiesBannerPreference", "#cookiesBannerChoices input:checked:not(:disabled)", "#cookiesBannerPreference button[onclick^='submitCookies()']", "#cookiesBanner.cookiesBannerHide", '"tracking":false', "#cookiesInfo #closeCookies", "#cookiesInfo", ".cookiesjsr-banner.active", ".cookiesjsr-banner", ".cookiesjsr-banner.active .denyAll", ".cookiesjsr-banner.active .cookiesjsr-settings", ".cookiesjsr-layer", ".cookiesjsr-layer .denyAll", ".cookiesjsr-layer .cookiesjsr-switch.active input", ".cookiesjsr-layer .save", "#cookiesModal .modal-footer #cookiesModalAcceptSelected", '#cookiesModal input[name="cookies_google"]', "#cookiesModal .modal-dialog", '#cookiesModal input[name="cookies_google"]:checked', "#cookiesModal #cookiesModalAcceptSelected", "cookiePolicy=sessions", "cookiePolicy=sessions%2Cgoogle", "#cookiesplus-modal-container", "#cookiesplus-overlay", "#cookiesplus-modal #cookiesplus-form", "#cookiesplus-modal", "#cookiesplus-modal .cookiesplus-reject", "#cookiesplus-modal .cookiesplus-more-information", "#cookiesplus-modal .cookiesplus-save", "C_P_DISPLAY_MODAL%22%3Afalse", "#CookieConsent dialog.cookie-popup", "#CookieConsent dialog.cookie-popup #cc-b-custom", "#CookieConsent .actions input[type=checkbox]:checked", "#CookieConsent #cc-b-custom", ".cky-overlay,.cky-consent-container", ".cky-consent-container", ".cky-consent-container [data-cky-tag=reject-button]", ".cky-consent-container [data-cky-tag=donotsell-button]", ".cky-modal [data-cky-tag=optout-option-toggle]", ".cky-modal [data-cky-tag=optout-option-toggle]:not(:checked)", ".cky-modal [data-cky-tag=optout-confirm-button]", ".cky-modal [data-cky-tag=optout-success]", ".cky-modal [data-cky-tag=optout-close]", ".cky-consent-container [data-cky-tag=settings-button]", ".cky-modal-open input[type=checkbox],.cky-consent-bar-expand input[type=checkbox],.cky-modal-open [data-cky-tag=detail-save-button],.cky-consent-bar-expand [data-cky-tag=detail-save-button]", ".cky-modal-open input[type=checkbox]:checked,.cky-consent-bar-expand input[type=checkbox]:checked", ".cky-modal [data-cky-tag=detail-save-button],.cky-consent-bar-expand [data-cky-tag=detail-save-button]", ".cky-consent-container,.cky-overlay,.cky-consent-bar-expand", "advertisement:no", "#cky-consent.cky-consent-bar", "#cky-consent #cky-btn-reject", "#cky-consent #cky-btn-settings", "#cky-btn-custom-accept", "input[id^=cky-checkbox-category]:checked:not(:disabled)", "cky-action=yes", "cky-consent=no", "cookieyes-analytics=no", ".elementor-popup-modal:has(.cookiez-footer)", ".cookiez-footer [data-cookiez-action]", '.cookiez-footer [data-cookiez-action="deny-all"]', "%22analytics%22%3Afalse", "dialog:has(#cookie-banner-2025-form)", "#cookie-banner-2025-form", 'button[form="cookie-banner-2025-form"][name="accept_cookie"]', 'button[form="cookie-banner-2025-form"][name="accept_cookie"][value="selection"]', ".c-cookie-banner.c-custom-sheet", '[id^="monetate_selectorHTML"] > .gdprCookieBanner', ".c-cookie-banner.c-custom-sheet .c-cookie-banner__close-button", '[id^="monetate_selectorHTML"] > .gdprCookieBanner .gdprCookieBannerCloseBtn', '.c-cookie-banner.c-custom-sheet, [id^="monetate_selectorHTML"] > .gdprCookieBanner', ".cookiealert", ".configurecookies", ".confirmcookies", "#gdprMessage button.gdpr-review", "[id^=div-transcend-cookie]", "#gdprMessage", "#btn-transcend-cookie-confirm, #btn-transcend-save", "#analyticsNo:not(:checked), #functionalNo:not(:checked), #targetingNo:not(:checked)", "#analyticsCookieCheckbox:checked, #preferenceCookieCheckbox:checked", "#elc-cookie-consent", "#elc-cookie-consent #elc-cookie-consent-form", "#elc-cookie-consent:not(.elc-hidden) #elc-cookie-consent-form", '#elc-cookie-consent-form input[type=checkbox][name^="group-"]:checked:not([disabled]):not([name$="-necessary"])', "#elc-accept-link", "#elc-detail-link", "#elc-save-link, #elc-accept-link", "#elc-save-link", "#elc-cookie-consent.elc-hidden", "#ct-ultimate-gdpr-cookie-popup", "#ct_ultimate-gdpr-cookie-reject a,#ct-ultimate-gdpr-cookie-reject", "#ct-ultimate-gdpr-cookie-change-settings", '#ct-ultimate-gdpr-cookie-modal-slider-form input[type=radio][value="1"], #ct-ultimate-gdpr-cookie-modal-slider-form input[type=radio][value="2"]', "#ct-ultimate-gdpr-cookie-modal-body .save", "ct-ultimate-gdpr-cookie=", "#cookiebar", "#cookiebar .cookiebar-content", "#cwarn-box", "#cwarn-box #cwarn-button", ".cookies-consent:has(.cookies-consent-accept-needed)", ".cookies-consent .cookies-consent-accept-needed", ".cookies-consent .cookies-inner", "cookiesConsentAllowedServices=", ']},{"type":', ".cookie-consent-banner:has(#FormCookieConsentAll)", ".cookie-consent-banner #FormCookieConsentAll", ".cookie-consent-banner .btn-cookie-consent-settings", ".modal-cookie-consent .btn-cookie-consent-modal-disallow-all", ".cookie-consent-banner", ".cookie-notice.js-cookie-modal", ".cookie-notice.js-cookie-modal .js-accept-minimal-cookies", "acceptCookiePolicy-marketing=false", 'div[class*="CookiePopup__desktopContainer"]:has(div[class*="CookiePopup"])', 'div[class*="CookiePopup__desktopContainer"]', ".cookie-banner.show .cookie-banner__content-all-btn", ".cookie-banner__content-essential-btn", "#CookiePolicy", "#CookiePolicy #cookie-policy-accept-button", "#CookiePolicy .cookie-policy-consents-area input[type=checkbox]:checked:not(:disabled)", "DDCookiePolicy-consent-tracking=false", "[data-module=cookie-consent-banner]", "[data-module=cookie-consent-banner] #button-accept-necessary[data-consent-set]", "[data-module=cookie-consent-banner] .cookie-consent-banner", "[data-module=cookie-consent-banner] #button-accept-necessary", "dbconsent=0.1.", ".dg-consent-banner", "datagrail_consent", "#datasign_cmp__host", "#cookieconsent.cookie-overlay .cookie-buttons #cookie-deny", '#cookieconsent.cookie-overlay[style*="visibility: visible"] #cookie-deny', "#cookieconsent.cookie-overlay #cookie-deny", "CLC_GDPR=AB%3A", "#cookie-notice", "#cookie-notice :is(#cn-refuse-cookie, #cn-close-notice)", "#cookie-notice.cookie-notice-visible #cn-notice-buttons", "#cookie-notice #cn-refuse-cookie", "#cookie-notice #cn-close-notice", "#cookie-notice .cookie-notice-container", "cookie_notice_accepted=false", "#didomi-popup,.didomi-popup-container,.didomi-popup-notice,.didomi-consent-popup-preferences,#didomi-notice,.didomi-popup-backdrop,.didomi-screen-medium", "#didomi-host", '#didomi-popup, #didomi-notice, .didomi-popup-notice, .didomi-notice-banner, #didomi-host:not([aria-hidden="true"]):not(:empty)', "#didomi-notice-disagree-button,.didomi-continue-without-agreeing", "#acceptationCMPWall", "#didomi-notice-learn-more-button", "#btn-toggle-disagree", "#diffuse-cookie-notice", "#diffuse-cookie-notice .dcn-bar", "#diffuse-cookie-notice .dcn-bar .diffuseCookieDenyAll", "#diffuse-cookie-notice .dcn-bar .diffuseCookieSettings", "#diffuse-cookie-notice .dcn-modal", "#diffuse-cookie-notice .dcn-modal .dcn-cbl-on:not(.dcn-cbl-locked)", "#diffuse-cookie-notice .dcn-modal .diffuseCookieAcceptSelection", "%3Atrue", "dnt-cookies", "CONSENT=OFF", "#cc_modal_dialog", "#cc_modal_window", "#cc_section_quick_info #cc_accept_all_cookies", "#cc_reject_all_cookies", "#cc_manage_cookies", "#cc_reject_all", "#cc_save_preferences", "#cc_section_quick_info", "cookie_consent=", 'script[src*="/style/dlibra/"]', ".cookies > p > button.cookies__button", "AcceptCookie=true", '[data-project="mol-fe-cmp"]', '[data-project="mol-fe-cmp"] [class*=footer]', '[data-project="mol-fe-cmp"] button[class*=basic]', '[data-project="mol-fe-cmp"] div[class*="tabContent"]', '[data-project="mol-fe-cmp"] div[class*="toggle"][class*="enabled"]', "#mol-ads-cmp-iframe, div.mol-ads-cmp > form > div", "div.mol-ads-cmp > form > div", "div.mol-ads-ccpa--message > u > a", ".mol-ads-cmp--modal-dialog", "a.mol-ads-cmp-footer-privacy", "button.mol-ads-cmp--btn-secondary", "#eufemia-portal-root .cookie-popup", ".cookie-popup #cookie-popup-strictlyNecessary", "consentData=sn", '[data-testid="cookie-banner"][class*="cookie-banner-styles__StyledCookieBanner"]', '[data-testid="cookie-banner"] button[kind="BUTTON/FLAT_SECONDARY"]', "cookie_tracking_enabled", "#pg-shadow-host-dom", ".cmp-popup_popup", ".cmp-app_gdpr", ".cmp-popup_popup .cmp-intro_rejectAll", ".cmp-popup_popup .cmp-details_rejectAll", "lpubconsent=", "#rgpd-popup", "#rgpd-custom-popup", "#rgpd-popup #rgpd-continue-without-accepting", "#rgpd-continue-without-accepting", "consentLevel=none", "#drupalorg-crosssite-gdpr", ".no", '#d-notification-bar[data-rule-type="notification"]:not([data-rule])', '#d-notification-bar[data-rule-type="notification"]:not([data-rule]) .notification-dismiss', '.showing-message--shown > #d-notification-bar[data-rule-type="notification"][data-visible]:not([data-rule])', ".showing-message--shown > #d-notification-bar:not([data-rule]) .notification-dismiss", ".showing-message--shown > #d-notification-bar:not([data-rule])", "div[data-testid=cookie-consent-modal-backdrop]", "div[data-testid=cookie-consent-message-contents]", "button[data-testid=cookie-consent-adjust-settings]", "button[data-testid=cookie-consent-preferences-save]", "cc_functional=0", "cc_targeting=0", "#cookiemodal", "#cookiemodal #cookie-accepted-form", "#cookiemodal #cookie-change-desc", "#cookiemodal .cookie-form input[type=checkbox]:checked:not(:disabled)", "analytics_storage=false", "ad_storage=false", ".cookieconsent form .hidden_accept_all", ".cookieconsent:not(.hidden) form input[type=submit]", "#cookie-consent-deny", ".cookieconsent form button.button--textual, #cookie-consent-button", ".cookieconsent .hidden_accept_all[value=no]", "#cookie_category_marketing:checked", "#cookie_category_marketing", ".cookieconsent form input[type=submit]", "cookie_category_marketing=on", "#eagerly-tools-cookie", "#eagerly-tools-cookie-overlay", "#eagerly-tools-cookie.active .cookie-fb-content", "#eagerly-tools-cookie .cookie-fb-scroller input[type=checkbox]:checked:not(:disabled)", "#eagerly-tools-cookie .cookie-fb-bottom :is(.js-btn-ok, .btn-ok)", "#easycookieinfo", "#cookieBox .block-cookie-consent", "#cookieBox #consentButton.block-cookie-consent--btn", "#cookieBox", "#econda-pp2-banner", "#econda-pp2-banner-modal", "#econda-pp2-banner, #econda-pp2-banner-modal", "#econda-pp2-banner-reject-all-channels-button", 'econda.privacy.protection={"profile":{"state":"DENY"', "#emCookieWrapper", "#emCookieWrapper #emCookieBanner", "#emCookieBanner", "#emCookieBtnConfigure", "#emCookieTool #emCookieBtnRejectAll", "#emCookieFrame", "EducaMadrid_LSSI_Cookies=0", ".cookieNoticeWrapper", ".cookieNoticeWrapper #cookieConsentFormAccept input[name^='ee:cookies_']", ".cookieNoticeWrapper #cookieConsentFormAccept", ".cookieNoticeWrapper #cookieConsentFormDecline button", ".cookieNoticeWrapper #cookieConsentFormAccept button", "div[role='dialog'][aria-modal='true']:has([data-qa='accept-all-button'])", "[data-qa='accept-all-button']", "[data-qa='reject-non-essential-button']", "#ensNotifyBanner", ".ensModal", "#ensNotifyBanner[style*=block]", "#ensModalWrapper[style*=block]", "#ensNotifyBanner #ensRejectAll,#ensNotifyBanner #ensRejectAllConsent,#ensNotifyBanner #rejectAll,#ensNotifyBanner #ensRejectBanner,#ensNotifyBanner .rejectAll,#ensNotifyBanner #bannerRejectButton,#ensNotifyBanner #ensRejectAds,#ensNotifyBanner #btn-accept-essential-cookies", "#ensOpenModal,#modalOpenButton,#bannerOpenModal", ".ensCheckbox:checked:not(:disabled)", "#ensSave,#modalSaveButton,#bannerSave", "#ensCloseBanner", "#ez-cookie-notification", "#ez-cookie-notification__decline", "#notificationManagerLink", "#ez-reject-modal", "cookies-marketing=false", "epaas-consent-drawer-shell", "%22advertising%22%3A0", "#emgCookieConsentOverlay", "#emgCookieConsentInitialPopup", "#emgCookieConsentInitialPopup button.minimal", '#cookiebar[data-cookie-name="eset_cookie-bar"]', '#cookiebar[data-cookie-name="eset_cookie-bar"] #cc-accept', "#cookiebar #cc-decline", "#cookiebar #cc-manage", "#cookiebar #cc-accept2", "#cookiebar .cc-category input[type=checkbox]:checked:not(:disabled)", '"marketing":0', "#et-consent-overlay", "#et-consent-overlay-bg", "#et-consent-overlay button[data-editor=buttonDeny]", "et_allow_cookies=0", "#gdpr-single-choice-overlay", "#gdpr-privacy-settings", "button[data-gdpr-open-full-settings]", ".gdpr-overlay-body input", ".pea_cook_wrapper,.pea_cook_more_info_popover", ".pea_cook_wrapper", "euCookie", "body.eu-cookie-compliance-popup-open", ".eu-cookie-compliance-banner .decline-button, .eu-cookie-compliance-banner .accept-necessary, .eu-cookie-compliance-save-preferences-button", "cookie-agreed=2", ".eupopup-container", ".eupopup-container > .eupopup-closebutton", "#ez-cookie-dialog-wrapper", "#ez-manage-settings", "#ez-cookie-dialog input[type=checkbox]", "#ez-cookie-dialog input[type=checkbox]:checked", "#ez-save-settings", "ez-consent-tcf", '[data-controller="cookie-message"]', '[data-controller="cookie-message"] [data-level="minimal"]', "cookies=nok", "cookies=minimal", ".aspire-banner.marketing-cookies-banner", '[data-trk-id="reject-marketing-cookies"]', "mktck=false", "#fast-cmp-root", "iframe#fast-cmp-iframe", "fastCMP-", "iframe.ck_if", "iframe.ck_if[title='Cookie center']", "html.ck_panel iframe.ck_if[style*='visibility: visible']", "html.ck_panel iframe.ck_if", "fedex-gdpr", "fedex-gdpr div", "fedex-gdpr .fxg-gdpr__reject-all-btn", "_svs=", ".cookies-container:has(.accept-necessary-only-btn)", ".cookies-container .cookies-banner-non-essential .accept-necessary-only-btn", ".cookies-container", ".cookies-container .accept-necessary-only-btn", ".cookies-container .accept-hide-message-btn", ".cookies-accepted-container #closeStatusPopUp", "_website_cookie_essentialCookies=essentialCookies", "_website_cookie_usageCookies=", 'astro-island[component-url*="CookiesBanner"] .cookies', 'astro-island[component-url*="CookiesBanner"] .cookies button.cookies__buttons--settings', 'astro-island[component-url*="CookiesBanner"] .cookies input[name="targeting"]:checked', 'astro-island[component-url*="CookiesBanner"] .cookies input[name="targeting"]', 'astro-island[component-url*="CookiesBanner"] .cookies button.cookies__buttons--accept', "cookiesPreferences=[%22necessary%22]", "#cookie-advice.fv-cookie-advice", "#cookie-advice .fv-cookie-advice__message a:not([href]), #cookie-advice .fv-cookie-advice__message button", ".fv-cookies-management", ':is(fv-cookies-management-sheet, .modal-content):has(.fv-cookies-management) button[class*="--primary"]', "fever_cookies_configuration", "#fides-overlay, #fides-overlay-wrapper", "#fides-overlay-wrapper #fides-banner", "#fides-overlay-wrapper .fides-banner-hidden", ".fides-banner-button", "#fides-banner .fides-reject-all-button", ".fides-banner:not(.fides-banner-hidden) .fides-reject-all-button", "#fides-banner .fides-manage-preferences-button", ".fides-modal-content .fides-reject-all-button", ".js-cookie-consent:has(.cookie-consent__accept-essential)", ".js-cookie-consent .cookie-consent__accept-essential", "[fs-consent-element=banner]", "[fs-consent-element=banner] [fs-consent-element=allow]", "[fs-consent-element=banner] [fs-consent-element=deny]", "[fs-consent-element=banner], [fs-consent-element=root]", "#first-visit.first-visit", "#first-visit.first-visit a.js-first", ".fp-cookie-modal", ".fp-cookie-modal .cookie-essentials", ".fp-cookie-modal a.cookie-essentials", "cookie_consent=essentials", ".flatsome-cookies", ".flatsome-cookies--active .flatsome-cookies__accept-btn", ".flatsome-cookies--active", '[class*="_cookieconsent-banner--open_"]', '[class*="_cookieconsent-banner_"] [class*="_cookieconsent__button-container_"]', '[class*="_cookieconsent-banner--open_"] [class*="_cookieconsent--initial_"]', '[class*="_cookieconsent--initial_"] [class*="_cookieconsent__button-container_"] > button:nth-child(2)', "%22analyticsConsent%22%3Afalse%2C%22thirdPartyConsent%22%3Afalse%2C%22surveyConsent%22%3Afalse", ".dse-cookie-disclaimer", ".dse-cookie-disclaimer .cta-decline-cookies", ".dse-cookie-disclaimer.cookie-visible:not(.cookie-hidden) .cta-decline-cookies", ".dse-cookie-disclaimer.cookie-hidden", "%22targeting%22%3Afalse", ".fc-consent-root,.fc-dialog-container,.fc-dialog-overlay,.fc-dialog-content", ".fc-consent-root", ".fc-dialog-container", ".fc-cta-do-not-consent,.fc-cta-manage-options", ".fc-li-only-placeholder", ".fc-preference-consent:checked,.fc-preference-legitimate-interest:checked", ".fc-confirm-choices", '[class*="gcb-modal-overlay"]', "[data-gcb-modal-show-prefs]", "[data-gcb-modal-save]", "gcbcl=a3|m3", ".cookiebanner1", "script[src*='consent.gameforge.com/']", ".cookiebanner1 .cookiebanner4 > button.cookiebanner6", ".cookiebanner1 .cookiebanner2", ".cookiebanner1 .cookiebanner11", ".cookiebanner1 .cookiebanner11 > button:last-child", "gf-cookie-consent-", "=|0|", 'html[lang^="ja"] #gdpr:has(> button:only-of-type)', 'html[lang^="ja"] #gdpr > button:only-of-type', ".overlay_bc_banner", 'a[href="https://gdpr-legal-cookie.myshopify.com/"]', ".overlay_bc_banner .banner-body", ".overlay_bc_banner *[data-cookie-save]", "#__pb-cookie", "#__pb #__pb-cookie", "#__pb-cookie.display", "#__pb-cookie .__pb-cookie_button_only_necessary", "#__pb-cookie .__pb-cookie_button_settings", "#__pb-settings", "#__pb-settings input.form-check-input:checked", "#__pb-settings .__pb-cookie_button_accept", "#cookieConsentWidgetDiv .cookie-banner", "#cookieConsentWidgetDiv #banner-cookie-consent-reject-all", "#banner-cookie-consent-reject-all", "g4m_cookie_consent=CONSENT_ESSENTIAL", '[data-test-id="cookie-banner-body"]', 'cookie-banner [data-test-id="cookie-banner-body"] [data-test-id="reject-button"]', '[data-test-id="cookie-banner-body"] [data-test-id="reject-button"]', "gov-core-cookies .cl-cookies", "gov-core-cookies .cl-cookies__bar", "gov-core-cookies .cl-cookies__bar gov-core-button:first-of-type button", "ngb-modal-window .modal-footer gov-core-button button", "gen_consent=", ".cdk-global-overlay-wrapper:has(pf-confirm-mat-modal a[href*='getproven.com/privacy'])", "pf-confirm-mat-modal a[href*='getproven.com/privacy']", "#cookie-container .navbar-cookies", "#cookie-container #accetta-cookie .cookie-container-setting", "#cookie-container", "#accetta-cookie input[type=checkbox]:checked:not(:disabled)", "#accetta-cookie .accept", "acconsenti-cookie=1", ".cookie-banner[data-available-cookies]", ".cookie-banner[data-available-cookies] .reject-cookies", "CookieConsent=[]", "#gtm_privacy", "pwinteraction=", "[data-aid=FOOTER_COOKIE_BANNER_RENDERED]", "[data-aid=FOOTER_COOKIE_BANNER_RENDERED] [data-aid=FOOTER_COOKIE_DECLINE_RENDERED]", "[data-aid=FOOTER_COOKIE_BANNER_RENDERED][style*=transform]", 'a[href^="https://policies.google.com/technologies/cookies"', 'form[action^="https://consent.google."][action$="/save"],form[action^="https://consent.youtube."][action$="/save"]', 'form[action^="https://consent.google."][action$="/save"]:has(input[name=set_eom][value=true]) button,form[action^="https://consent.youtube."][action$="/save"]:has(input[name=set_eom][value=true]) button', ".glue-cookie-notification-bar", ".glue-cookie-notification-bar__reject", ".HTjtHe#xe7COe", '.HTjtHe#xe7COe a[href^="https://policies.google.com/technologies/cookies"]', ".HTjtHe#xe7COe button#W0wltc", "SOCS=CAE", ".govuk-cookie-banner__message", ".govuk-cookie-banner__message .govuk-button", "#gravitoCMPRoot", "#gravitoCMPRoot [id^=gravito][id$=-layer1-accept-all]", "#gravitoCMPRoot [id^=gravito][id$=-layer1-reject-all]", "#gravitoCMPRoot [id^=gravito][id$=-layer1-settings]", "#gravitoCMPRoot [id^=gravito][id$=-layer2-reject-all]", "gravitoData", '[data-test="modal-cookie-bottom-bar"], [data-test="section-cookieModal"]', '[data-test="modal-cookie-bottom-bar"]', '[data-test="button-customizeCookie"]', '[data-test="modal-cookie-customize"]', '[data-test="modal-cookie-customize"] input[role="switch"]:checked:not([disabled])', '[data-test="modal-cookie-customize"] [data-test="button-submit"], dialog:has([data-test="modal-cookie-customize"]) [data-test="button-submit"]', '[data-test="section-cookieModal"] [data-test="button-customize"]', '[data-test="section-customizeModal"]', '[data-test="section-customizeModal"] input[data-ischecked="true"]:not([disabled])', '[data-test="section-customizeModal"] [data-test="button-save"]', "gpc_ad=0", "cp_advertisement=0", "#gr-consent-layer-area", "#gr-consent-layer", "#gr-btn-more-options", "#gr-btn-legitimate-partners", "#toggleLegitimateCheckbox", "#toggleLegitimateCheckbox:not(:checked)", "#gr-btn-save-exit-primary", ".bhh-cookie-banner", ".bhh-cookie-banner .bhh-cookie-banner__btn--close, .cookie-banner .cookie-banner__btn--close", ".bhh-cookie-banner__btn--close, .cookie-banner__btn--close", "#cookiebanner.cookiebanner__restrictive", "#c-cookiebanner.c-cookiebanner", ".js-banner__container .js-banner-button-confirm-selection", ".js-banner__container input.js-banner-control__element:checked:not(:disabled)", "gsbbanner=closed", "#guesty-be-root .cookies__block", "#guesty-be-root", "#guesty-be-root .cookies__block .cookies__btn--decline", "cookiePolicyAccepted=0", "div[class^='overlay-cookie-uuid-']", "div[class^='overlay-cookie-uuid-'] [class^='button-container-cookie-uuid-']", "div[class^='overlay-cookie-uuid-'] [class^='accordion-container-cookie-uuid-']", "div[class^='overlay-cookie-uuid-'] button[class*='btn-default-cookie-uuid-']", "div[class^='overlay-cookie-uuid-'] input[class^='cookie-uuid-'][id$='-legitimate-interest']", "div[class^='overlay-cookie-uuid-'] input[class^='cookie-uuid-'][type='checkbox']:checked", ".cookies-bar__close-button", "%22advertising%22:0", "allow_marketing=deny", "#preferences-box", "#preferences-box [data-js=open-settings]", "#preferences-box [data-js=select-preferences]", "#preferences-box [data-js=preferences-form] input[value=mandatory]", 'consent_status={"consent_all":"denied","consent_personalized":"denied"', "shb-cookie-consent", 'shb-cookie-consent [data-testid="CookieConsent_modal"]', 'shb-cookie-consent [data-testid="cookie_consent_accept_only_necessary_btn"]', "shb-cookies-consent=false", "#modal-host > div.no-hash > div.window-wrapper", "#modal-host > div.no-hash > div.window-wrapper, div[data-testid=qualtrics-container]", '#modal-host > div.no-hash > div.window-wrapper > div:last-child a[href="/privacy-settings"]', "div#__next", "#__next div:nth-child(1) > button:first-child", '.cookie-info:has(a[href*="close-policy.cfm"])', '.cookie-info a[href*="close-policy.cfm"]', ".cookie-modal .cookie-accept-btn", ".cookie-modal .js-cookie-reject-btn", "cookies_rejected=1", ".hkd-cookie-banner", ".hkd-cookie-banner .hkd-cookie-banner-button-group", "next-route-announcer", ".hkd-cookie-banner-button-group > button:nth-of-type(2)", "cookie-consent=essential", ".cookieModalContent", "#cookie-banner-overlay", "#manageCookie", ".cookieSettingsModal", "#AOCookieToggle", "#AOCookieToggle[aria-pressed=true]", "#TPCookieToggle", "#TPCookieToggle[aria-pressed=true]", "#updateCookieButton", ".cookie-consent-popup", ".cookie-consent-popup #banner-accept-btn", ".cookie-consent-popup #banner-reject-btn", ".cookie-consent-popup #banner-cookie-setting-btn", ".settings-page #settings-only-required-btn", "groups=C0001:1,C0002:0,C0003:0,C0004:0", ".cbanner-govuk-cookie-banner", ".cbanner-govuk-cookie-banner button[value=reject]", ".cbanner-govuk-cookie-banner button:not([value])", "%22measurement%22:false", "dialog[data-cookie-consent]:has([data-cookie-accept])", "dialog[data-cookie-consent] [data-cookie-accept]", 'a[data-cookie-accept="functional"]', "#cookie-selector", '#cookie-selector [data-testid="CookiesCheckbox__DeclineNonEssential"]', "__mcons=false", "__acons=false", '[data-test="cookie-consent-notification"]', '[data-test="cookie-consent-notification"] [data-test="accept-button"]', '[data-test="cookie-consent-notification"] [data-test="dismiss-button"]', '[data-test="cookie-consent-notification"] [data-test="settings-button"]', '[data-test="cookie-consent-modal"] [data-test="accept-button"]', "#hu.hu-wrapper", "#hu.hu-visible", "#hu-cookies-save", "#hs-eu-cookie-confirmation", "#hs-eu-decline-button", "#hs-eu-close-button", "#JOISH987_bar_holder", "#JOISH987_bar_holder #JOISH987_bar", "#JOISH987_rejectBar_btn", "#JOISH987_inlineRejectBar_btn", "#cookie-modal", "#cookie-modal #gdpr-customize-settings", "#cookie-modal #gdpr-reject-all", "#cookie-modal #gdpr-save-settings", "IB_PRIV_PREFS=1%7C1%7C1", "body > #idxrcookies", "body > #idxrcookies #idxrcookiesKO", "#gdpr-cookie-message", "#gdpr-cookie-message #gdpr-cookie-advanced", "#gdpr-cookie-types", "#gdpr-cookie-types input[type=checkbox]:checked:not(:disabled)", "#gdpr-cookie-message #gdpr-cookie-accept2", "#gdpr-cookie-message #gdpr-cookie-accept", "#handlebars-cookie-confirm", "#gdpr-cookie-message #gdpr-cookie-accept-post", "cookieControl=true", "#toast-container.ck-tstr #ck_set", "#cmp_settings .cmp__button--reject", "cmp_ck=3", "body.page-ipublic #div-cookies", 'body.page-ipublic #div-cookies button[onclick="acceptCookies()"]', "#ipc_cookie_confirmation", "#ipc_cookie_confirmation .cookie_confirmed_and_close", "ipc_cookie=confirmed", ".base-footer > .cookie-policy", ".base-footer > .cookie-policy .btn-cookie", "[data-in2-modal-root-node]", "[data-in2-modal-root-node] .in2-modal-deny-button", "[data-in2-modal-root-node] .in2-modal-checkbox__input:checked:not(:disabled)", "[data-in2-modal-root-node] [data-in2-modal-save-button]", "in2cookiemodal-selection=[%22essential%22]", "cookie-consent-element", '"isSelected":false', '.q-notifications .q-notification:has(a[href="/cookie-policy"])', '.q-notifications .q-notification a[href="/cookie-policy"]', "module-rgpd-component", "cookieConsent=consent_1", ".accept-cookie", ".accept-cookie > .accept-cookie-inner", "xpath///*[contains(@class, 'accept-cookie')]//*[contains(text(), 'Decline')]", "cookie-pref=rejected", '[data-preact="cookie-banner/index"] .js-cookiebar', '[data-preact="cookie-banner/index"] .js-cookiebar .js-cookie-deny', ".gebaeck-notice", ".gebaeck-notice .gebaeck-notice__buttons", ".gebaeck-notice button.btn--privacy-notice-only-necessary", ".gebaeck-notice button.btn--gebaeck-notice-config, .gebaeck-notice button.btn--privacy-notice-config", ".gebaeck-notice-popup", ".gebaeck-notice-popup input[type=checkbox]:checked:not(:disabled)", ".gebaeck-notice-popup button.btn--gebaeck-notice, .gebaeck-notice-popup button.btn--privacy-notice", "privacy=", "form.rodo-popup", "form.rodo-popup button.rodo-popup-main-settings", "form.rodo-popup button.rodo-popup-reject", "#stopka-polityka", "#stopka-polityka #stopka-polityka-zamknij", "#stopka-polityka-zamknij", "prywatnosc=tak", '#elGuestTerms[data-role="guestTermsBar"]', "xpath///div[@id='elGuestTerms'][@data-role='guestTermsBar'][.//a[@data-action='dismissTerms']][not(.//form)][count(.//a[contains(@class,'ipsButton')] | .//button) <= 1][contains(translate(., 'COKIE', 'cokie'), 'cookie')]", ".privacy-consent--backdrop", ".privacy-consent--modal", ".footer-config-link", "#confirmSelection", '.m-cookie-banner[data-id="cookie-banner"]', '.m-cookie-banner[data-id="cookie-banner"] [data-id="cookie-manage"]', "#ccc-icon", "#ccc-module .ccc-reject-button", '"optionalCookies":{', '"revoked"', "#iubenda-cs-banner", ".iubenda-cs-accept-btn", "#iubenda-cs-banner .iubenda-ccpa-opt-out", ".iubenda-cs-reject-btn", ".iubenda-cs-customize-btn", ".iub-btn-reject", "#iubFooterBtn", "#iubenda-alert .iubenda-button-confirm", "#iubenda-cs-banner .iubenda-cs-close-btn", "uspString%22%3A%221YY-", "body.cookies-request #cookie-bar", "body.cookies-request #cookie-bar .disallow-cookies", "cookie_permission_granted=no", ".js-jcookie", ".js-jcookie .js-jcookie-allow", ".js-jcookie.active", ".widget_eu_cookie_law_widget", "#jix-cookie-consent-modal", "#jix-cookie-consent-modal input.jix-cookie-consent-selection:checked:not(:disabled)", "#jix-cookie-consent-accept-selected", "div[class^=pecr-cookie-banner-]", "button[data-test^=manage-cookies]", "label[data-test^=toggle][class*=checked]:not([class*=disabled])", "button[data-test=save-preferences]", '[data-testid="jkl-cookie-consent-godta"]', '[data-testid="jkl-cookie-consent-nekt"]', '[id^="jkl-cookie-modal"] input[type="checkbox"]:checked', '"statistics":"denied"', ".js-cookie-consent:has(.cookie-consent__settings-save)", ".js-cookie-consent .cookie-consent__settings-save", ".js-cookie-consent.cookie-consent--open .cookie-consent__button--secondary", ".js-cookie-consent .cookie-consent__button--secondary", ".js-cookie-consent .cookie-consent__option-checkbox:checked", ".js-cookie-consent.cookie-consent--open", ".consent-banner:has(.consent-close-icon-wrapper)", '.consent-banner a.consent-close-icon-wrapper[data-pt-name="consent-closed"]', "cbc=true", ".js-cp-box", ".js-cp-box .js-cp-accept-all-button", ".js-cp-box .js-cp-reject-all-button", ".js-cp-box .js-cp-settings-button", ".js-cp-box .js-cp-settings-box", ".js-cp-box .new-cp-content-settings-options-one-cookie.on:not(.disabled)", ".js-cp-box .js-cp-accept-selected-button", "cookiepolicyinfo_new", ".cookie-bar", ".cookie-bar .cookie-bar__message,.cookie-bar .cookie-bar__buttons", "cookies-state=accepted", '.ui-dialog[aria-describedby="cookies"]', "#cookies > #cookies-info #btnCookiesAll", "#cookies-info #btnCookiesReject", "#cookies-info #btnCookiesOptions", "#cookies-options #btnCookiesSelected", '#cookies-options input[type="checkbox"]:checked', ".gdprPopup:has(> .gdprPopupContainer > .gdprButton)", "script#kasp-cookie-alert-js", ".gdprPopup > .gdprPopupContainer > .gdprButton", ".gdprPopup", ".kc-overlay", "#kconsent", ".kc-dialog", "#kc-denyAndHide", "#lanyard_root div[role='dialog']", "#ketch-banner", "#ketch-modal", "#lanyard_root [class*='ketch-fixed']", "#lanyard_root button[aria-label^='Reject' i], #ketch-banner button[aria-label^='Reject' i]", "#lanyard_root [aria-describedby=preference-description], #lanyard_root [aria-describedby=modal-description], #ketch-modal, #ketch-purposes-modal, #ketch-preferences", "#lanyard_root button[class*=confirmButton], #lanyard_root div[class*=actions_] > button:nth-child(1), #lanyard_root button[class*=actionButton], #ketch-modal button[aria-label='Confirm'], #ketch-purposes-modal button[aria-label='Confirm'], #ketch-modal button[aria-label='Save Settings'], #ketch-purposes-modal button[aria-label='Save Settings']", "#ketch-banner-button-tertiary", "#lanyard_root [aria-describedby=banner-description]", "#lanyard_root div[class*=buttons] > button[class*=secondaryButton], #lanyard_root button[class*=buttons-secondary], #ketch-banner-button-secondary", "#lanyard_root [id^='ketch-banner-buttons-container'] > button:only-child", "#lanyard_root [aria-describedby=preference-description],#lanyard_root [aria-describedby=modal-description], #ketch-preferences, #ketch-purposes-modal", "#lanyard_root button[class*=rejectButton], #lanyard_root button[class*=rejectAllButton], #ketch-modal button[aria-label='Reject All'], #ketch-purposes-modal button[aria-label='Reject All']", "#lanyard_root input[type=checkbox][role=switch][aria-checked=true]:not(:disabled)", "_ketch_consent_v1_", ".cc-container.cc-noselect", ".cc-container .cc-box .cc-section-landing .cc-btn-reject", ".cc-container.displayed .cc-section-landing", ".cc-container .cc-section-landing .cc-btn-reject", 'cookie_consent={"status":"rejected"', "#xp-cookie-banner-root dialog.xp-cookie-banner", "#xp-cookie-banner-root dialog.xp-cookie-banner[open]", '.xp-cookie-banner .xp-cookie-wrapper--buttons button[data-button="secondary_cta"]', "klp-marketing-consent=false", 'body.kolibri [data-testid="cookie-widget"]', 'body.kolibri [data-testid="cookie-widget"] button.k-button--primary + button.k-button--tertiary', "%22advertising%22%3Afalse%2C%22hasConsented%22%3Atrue", ".consentForm__container", ".consentForm__root .consentForm__acceptButton[data-accept-action=selected]", ".consentForm__root", ".consentForm__optOutItem input:checked", ".consentForm__acceptButton[data-accept-action=selected] button", "optOutAccepted=true", 'modal-box[data-options*="cookies"] .modal-object--cookie', ".modal-object--cookie [data-js-cookies-decline]", "#kit-ccm", "#kmcc-cookie-bar", "#kmcc-cookie-bar.kmcc-cookie-bar--visible", '#kmcc-cookie-bar .js-kmcc-extended-modal-button[data-url$="cookie_preferences"]', "#kmcc-accept-some-cookies", ".js-kmcc-btn-toggle:checked", "analyzing_cookie%22%3A%22false%22%2C%22marketing_cookie%22%3A%22false", ".cookie-settings-message", ".cookie-settings-background", ".cookie-settings-message #frmCookiesBanner", ".cookie-settings-message .cookie-settings-message-close-link", ".cookie-settings-message .cookie-settings-manage", "#frmCookiesBanner", "#frmCookiesBanner input[type=checkbox]:checked", ".cookie-settings-message .cookie-settings-message-save-link", "#_evh-ric", "#_evh-ric #btn_donotaccept", "__cpmtAccept=false", "#lab-consent", "#lab-consent .lab-consent-reject", "lab_analytics_consent=denied", ".privacy-notification[data-cookie-name]", ".privacy-notification[data-cookie-name] #js-privacy-notification-ok", "#js-privacy-notification-ok", ".cookie-consent.cc-banner", '#cc-settings-form input[name="cookie_consent_types[]"]', 'input[type=hidden][name="cookie_consent_types[]"] ~ div > button[type=submit]', 'input[type=hidden][name="cookie_consent_types[]"] ~ div > button[type=button]', "#cc-settings-form button[type=submit]", "#cc-settings-form input[type=checkbox]:checked:not(:disabled)", ".lcc-modal.js-lcc-modal-alert", ".lcc-modal.js-lcc-modal-alert .js-lcc-settings-toggle", ".lcc-modal.js-lcc-modal-settings", ".lcc-modal.js-lcc-modal-settings input[type=checkbox]:checked:not(:disabled)", ".lcc-modal.js-lcc-modal-settings .js-lcc-settings-save", "__cookie_consent=false", '.eu-popup:has(form[action$="/saveTheCookie"])', '.eu-popup form[action$="/saveTheCookie"] input[type=checkbox]', '.eu-popup:has(form[action$="/saveTheCookie"]) #rejectAllButton', '.eu-popup form[action$="/saveTheCookie"] input[type=checkbox]:checked:not(:disabled)', '.eu-popup:has(form[action$="/saveTheCookie"]) #saveButton', "#lw-widget-root #lw-banner", "#lw-widget-root #lw-banner.lw-banner-visible", "#lw-banner.lw-banner-visible .lw-consent-reject", "#lw-banner.lw-banner-visible", '"marketing":{"wanted":false}', "#jmdCookieConsentWrapper", "[data-cc-window] [data-cc-popup]", "[data-cc-window] [data-cc-popup] [data-cc-action=allow]", "[data-cc-window] [data-cc-popup] button[data-cc-action=dismiss]", "[data-cc-window] [data-cc-popup] [data-cc-action=close-popup]", "cookieconsent_status=dismiss", ".lia-cookie-banner-alert", ".lia-cookie-banner-alert .AjaxFeedback", ".lia-cookie-banner-alert a.lia-cookie-banner-alert-reject", "#p_p_id_com_liferay_cookies_banner_web_portlet_CookiesBannerPortlet_", 'script[src*="otSDKStub"]', "#_com_liferay_cookies_banner_web_portlet_CookiesBannerPortlet_declineAllButton", "CONSENT_TYPE_PERFORMANCE=false", ".darken-layer.open,.lightbox.lightbox--cookie-consent", "body.cookie-consent-is-active div.lightbox--cookie-consent > div.lightbox__content > div.cookie-consent[data-jsb]", ".cookie-consent__footer > button[type='submit']:not([data-button='selectAll'])", "#lgcookieslaw_banner,#lgcookieslaw_modal,#lgcookieslaw-modal,.lgcookieslaw-overlay,.lgcookieslaw_overlay", ".lgcookieslaw-reject-button-enabled,#lgcookieslaw-overlay,.lgcookieslaw-overlay", ".lgcookieslaw-reject-button-enabled .lgcookieslaw-reject-button", ".lgcookieslaw-reject-button-enabled", ".artdeco-global-alert[type=COOKIE_CONSENT]", ".artdeco-global-alert[type=COOKIE_CONSENT] button[action-type=DENY]", "#cookie-notice.fixed:has(#accept-cookies)", "#cookie-notice.fixed #accept-cookies", "#lta-cookies-banner", "#js-lta-cookie-modal-trigger", "#lta-cookies-modal", "#lta-cookies-form input[type=radio][id^=decline-]", "#js-lta-cookie-modal-set", "%22perf%22:false", "#macaron_cookie_box", "#macaron_cookie_box .macaronbtn", ".macaronbtn.refuse", ".macaronbtn.letmechoose", ".macaronbtn.letmechoose.open", "#cookie_description .paragraph", "#cookie_description input[type=checkbox]:checked:not(:disabled)", ".macaronbtn.confirmselection", "_deCookiesConsent", ".cookies-prompt__banner", ".cookies-prompt__wrapper .cookies-prompt__banner", ".cookies-prompt__banner_preview", ".cookies-prompt__banner_preview .cookies-prompt__actions-btn.btn-secondary", ".cookies-prompt__actions-wrap_details", ".cookies-prompt__banner_details .switch input:checked:not([disabled])", ".cookies-prompt__actions-wrap_details .cookies-prompt__actions-btn.btn-secondary", "cookies_consent=", ".mco-consent-pop-up", ".mco-consent-slide-up", ".mco-consent-slide-up__backdrop", ".mco-overlay:has(.mco-consent)", "#mco-consent", '[class*="mco-consent-"][class*="__button-decline"]', '"acceptedAll":false', "div[aria-labelledby=pwa-consent-layer-title]", "div[class^=StyledConsentLayerWrapper-]", "div[aria-labelledby^=pwa-consent-layer-title]", "button[data-test^=pwa-consent-layer-deny-all]", ".cookie-consent-simple-dialog", ".cookie-consent-simple-dialog .oo-ui-processDialog-actions-other a", ".cookie-consent-simple-dialog .oo-ui-processDialog-actions-other .oo-ui-buttonWidget:not(.oo-ui-flaggedElement-progressive) a", ".cookie-consent-detailed-dialog .oo-ui-processDialog-actions-primary a", ".cookie-consent-detailed-dialog input[type=checkbox]:checked:not(:disabled)", "cookieconsent_dialog_dismissed=dismissed", "#mensajeCookies", "#divConfigCookies", "#mensajeCookies .cajaCookies #btnAcceptAll", "#mensajeCookies #btnAcceptNoAllCookies", "#divConfigCookies #btnAcceptNoCookies", "#mensajeCookies .cajaCookies", "#mensajeCookies #btnConfigCookies", "cmm-cookie-banner", "#cookies-disclaimer-modal", "#cookies-disclaimer-modal #cookies-disclaimer-modal-settings", "#cookies-disclaimer-modal #cookies-disclaimer-modal-accept", "#cookies-disclaimer-modal-settings", "#cookies-settings-modal-header", "#cookies-settings-modal-statistics-switch:checked", "#cookies-settings-modal-confirm", '"googleAnalytics":false', '[data-testid="cookie-policy-dialog"]', '[data-testid="cookie-policy-manage-dialog"]', '[data-testid="cookie-policy-dialog"] [data-cookiebanner="accept_only_essential_button"], [data-testid="cookie-policy-manage-dialog"] [data-cookiebanner="accept_only_essential_button"]', '[data-cookiebanner="accept_only_essential_button"]', ".cmp_gdpr .fondo-cmp", ".cmp_gdpr #btn-gdpr-accept", ".cmp_gdpr #btn-gdpr-configure", ".cmp_gdpr .bt-final.footer button.bt-izq-m", "cmp-cl=AA", "#wcpConsentBannerCtrl", "#msCookieBanner", "#msCookieBanner.msCookieBanner", ".bottom-cookie-wrap:has(.js-bottom-cookie-close)", ".bottom-cookie-wrap .js-bottom-cookie-close", "#gdprCookieBar", "#gdprCookieBarOverlay", ".mst-gdpr__cookie-bar-wrapper", ".mst-gdpr__cookie-bar-overlay", ".mst-gdpr__cookie-bar .mst-gdpr__buttons,.mst-gdpr-cookie-bar-wrapper .mst-gdpr-buttons", ".mst-gdpr__buttons [data-trigger-settings=decline],.mst-gdpr__buttons button[\\@click=handleDecline],.mst-gdpr-cookie-bar-wrapper button[\\@click=handleDecline]", ".mst-gdpr__buttons [data-trigger-settings=decline]", ".mst-gdpr__buttons button[\\@click=handleDecline]", ".mst-gdpr-cookie-bar-wrapper button[\\@click=handleDecline]", ".mst-gdpr__buttons [data-trigger-settings=trigger]", ".mst-gdpr__buttons button[\\@click=openSettings]", "[data-mst-gdpr-settings-apply],.mst-gdpr__cookie-settings--success-btn,[x-on\\:click=save]", "input.checkbox[data-group-id]:not([disabled]):checked", "[data-mst-gdpr-settings-apply]", ".mst-gdpr__cookie-settings--success-btn", "[x-on\\:click=save]", "gdpr_cookie_consent=", "#cookies.modal:has(.aceptar-cookies)", "#configuracion-cookies.modal", '#cookies.modal button[data-bs-target="#configuracion-cookies"]', "#cookies.modal .aceptar-cookies", "#configuracion-cookies .aceptar-cookies", "#configuracion-cookies input[value=analytics-cookies]:checked", "#configuracion-cookies input[value=analytics-cookies]", 'div[role="dialog"]:has(button#necessary[role="switch"]):has(button#marketing[role="switch"]) [role="tablist"]', 'div[role="dialog"]:has(button#necessary[role="switch"]):has(button#marketing[role="switch"])', 'div[role="dialog"]:has(button#necessary[role="switch"]) div:not([role="tablist"]) > button:nth-child(3):last-child', "decided=true", "#__consent .md-consent__inner", "#__consent .md-consent__overlay", "#__consent form[name='consent'] .md-consent__settings input[type='checkbox']", "#__consent:not([hidden]) .md-consent__inner", "#__consent .md-consent__settings input[type='checkbox']:checked", "#__consent .md-consent__controls button.md-button--primary", "#cookiesdirective", "#cookiesdirective #explicitreject", 'dialog[data-testid="modal-dialog"]:has([data-testid="uc-button-decline"])', 'dialog[data-testid="modal-dialog"] [data-testid="uc-button-decline"]', "dialog[data-testid=accept-our-cookies-dialog]", "#banner-manage", "#pc-confirm", "#moove_gdpr_cookie_info_bar", "#moove_gdpr_cookie_info_bar:not(.moove-gdpr-info-bar-hidden)", ".moove-gdpr-infobar-reject-btn", "#moove_gdpr_cookie_info_bar .change-settings-button", "#moove_gdpr_cookie_modal", ".moove-gdpr-modal-save-settings", "#msg-cookie:has(#hide-msg-cookie)", "#msg-cookie #hide-msg-cookie", "#msg-cookie", "#cookie_consent_wrapper #cookie_consent.privacy_headsup", "#cookie_consent_wrapper #submit_privacy_consent1", "#cookie_consent #cookie_only_necessary2", "#gCookieConfirmToast", "#gCookieConfirmDimmed", "#gCookieConfirmToast .g-cookie-btn-deny", "applyCookie=%7B%22typeArr%22%3A%5B%22NECESSARY%22%5D", "#nhsuk-cookie-banner", "#nhsuk-cookie-banner__link_accept", ".bpPrivacy", ".bpPrivacy .bpPrivacy_button-close", ".govpl.cookie-bar-container", ".govpl.cookie-bar-container .cookiebar-close", ".govpl.cookie-bar-container:not(.cookie-bar-hide) .cookiebar-close", "cookie-message-bar=true", 'div.fixed.rounded-2xl.bg-background-secondary:has(> div.flex > button[class*="btn--skin-secondary___"] + button[class*="btn--skin-primary___"])', 'div.fixed.rounded-2xl.bg-background-secondary > div.flex > button[class*="btn--skin-secondary___"]:first-child', "r_consents=%7B%22required%22%3Atrue%2C%22analytics%22%3Afalse", ".js-ne-piwik-cookies", ".js-ne-piwik-cookies-overlay", ".js-ne-piwik-cookies .ne-piwik-cookies__btn-manage", ".js-ne-piwik-cookies .ne-piwik-cookies", ".ne-piwik-cookies__btn-manage", ".ne-piwik-modal__btn-check-choosen", ".ne-piwik-modal input.ne-piwik-modal__checkbox-input:checked:not(:disabled)", "ppms_privacy_", "%22analytics%22:{%22status%22:0", "ccm-notification", ".cookie-manager .modal-mask", ".cookie-manager .modal-mask .accept-cookie-settings", ".cookie-manager .modal-mask .decline-cookie-settings", '.cookie-manager .modal-mask input[type="checkbox"]:checked:not(:disabled)', ".cookie-manager .modal-mask .save-cookie-settings", "#privacy-banner.nyuuninav--privacy-banner", ".nyuuninav--privacy-banner .nyuuninav--privacy-banner__accept-button", ".nyuuninav--privacy-banner:not(.nyuuninav--privacy-banner--hide)", ".nyuuninav--privacy-banner__accept-button", "privacy_banner=", ".disc-cp--active", ".disc-cp-modal__modal", ".js-disc-cp-deny-all", ".js-occ-gdrp-modal", ".js-occ-gdrp-modal .js-occ-gdrp-modal__btn-modal-gdpr-settings", ".js-occ-gdrp-modal.is-visible .js-occ-gdrp-modal__btn-modal-gdpr-settings", ".js-occ-gdrp-modal .js-occ-gdrp-modal__btn-modal-gdpr-settings:not(.settings-active)", ".js-occ-gdrp-modal .js-occ-gdrp-modal__btn-modal-gdpr-settings.settings-active", ".js-occ-gdrp-modal .js-cookie-category:checked", "occ_gdpr_settings", ".tx-om-cookie-consent", ".om-cookie-panel[data-omcookie-panel]", ".active[data-omcookie-panel]", "[data-omcookie-panel-save=min]", "input[data-omcookie-panel-grp]:checked:not(:disabled)", "[data-omcookie-panel-save=save]", "#oc_cb_wrapper", "#oc_cb_wrapper #oc_cb_btn", ".legalmonster-cleanslate", ".legalmonster-cleanslate #lm-cookie-wall-container", "#lm-accept-necessary", "#o5-legal-banner-container", "#o5-legal-banner-container .o5-legal-banner-container__close", ".orejime-NoticePortal", ".orejime-BannerWrapper", ".orejime-banner-portal", ".orejime-Notice, .orejime-Banner, .orejime-banner-portal .fr-consent-banner, .orejime-ModalWrapper", ".orejime-Notice-declineButton, .orejime-Banner-declineButton, .orejime-banner-portal .fr-consent-banner", ".orejime-Notice-declineButton, .orejime-Banner-declineButton, .orejime-banner-portal .fr-consent-banner__buttons li:nth-child(2) button", ".orejime-ModalWrapper .orejime-AppToggles-disableAll, .orejime-ModalWrapper .orejime-PurposeToggles-disableAll", ".orejime-ModalWrapper .orejime-Modal-saveButton", "#CookieBanner.cookie-banner.drawer", "#CookieBanner.cookie-banner.drawer.opened", ".osano-cm-window,.osano-cm-dialog", ".osano-cm-window", ".osano-cm-dialog:not(.osano-cm-dialog--hidden)", ".osano-cm-denyAll,.osano-cm-deny", ".osano-cm-dialog--type_bar .osano-cm-dialog__close", ".cookieBanner--visibility", ".cookieBanner__wrapper", ".js_cookieBannerProhibitionButton", ".cookie-banner:has([data-ol-cookie-banner-set-consent])", "[data-ol-cookie-banner-set-consent]", "[data-ol-cookie-banner-set-consent=essential]", "oa", ".js-cookie-notice:has(#cookie_settings-form)", ".js-cookie-notice #cookie_settings-form", ".js-cookie-notice button[value=disable]", "#pandectes-banner", "pandectes-cmp", "#pandectes-banner .cc-deny", "#pandectes-banner .cc-settings", ".pd-cp-ui-save", ".pd-cp-ui-rejectAll", "#passle-cookie-control-container #cookie-control", "#passle-cookie-control-container", "#passle-cookie-control-container.cookie-control_state--open #cookie-control", "#cookie-control .cookie-control--decline", "passleCookieControl={%22Accepted%22:false", "#ccpaCookieContent_wrapper", "#ccpaCookieBanner", "#bannerDeclineButton", "type%3Dexplicit", "#gdprCookieBanner", "#gdprCookieContent_wrapper", "#overlay > .gdprBackgound", "#overlay > .gdprBackgound > .gdprTitle", "#overlay > .gdprBackgound > span[onclick]", "pxCookie=Y", "pe-cookies", "TC_PRIVACY=1%40", '#consent_block[widget="CookieTracker"]', '#consent_block [widget-attachpoint="basicAgree"]', ".cookie-wrapper .cookie-tip", ".cookie-wrapper .cookie-tip__container", ".cookie-wrapper .cookie-tip__text", ".cookie-wrapper .cookie-tip__button > .button", ".el-dialog.cookie-dialog", ".el-dialog.cookie-dialog .el-switch.is-checked", ".el-dialog.cookie-dialog .el-dialog__footer button.el-button--primary", "allow_analysis=%22false%22", "allow_analysis=false", "#PiwikPROConsentForm-container", ".PiwikPROConsentForm-overlay", "#ppms_cm_popup_overlay", "#PiwikPROConsentForm-container, #ppms_cm_popup_overlay #ppms_cm_reject-all, #ppms_cm_popup_overlay #ppms_cm_disagree", "#PiwikPROConsentForm-container, #ppms_cm_popup_wrapper", "#ppms_cm_reject-all, #ppms_cm_disagree", "#ppms_consent_form_success_note_button", "#PiwikPROConsentForm-reject-all", "#PiwikPROConsentForm-toggle-details", "#PiwikPROConsentForm-consents", "button.PiwikPROConsentForm-consent-checkbox[aria-checked=true]:not([aria-required=true]), input.PiwikPROConsentForm-consent-checkbox:checked:not(:disabled)", "#PiwikPROConsentForm-save", "%22status%22:0", "#cookies:has(> button.close > span.visuallyhidden)", "#cookies > button.close > span.visuallyhidden", "#cookies", "#cookieBanner", "#cookieBanner #openCookieBannerSettings", "#cookieBannerSettings #CookieBannerSettingsRejectAll", "#cookieBannerSettings", "hpa=true", "[data-cookie-bar]", '[data-cookie-bar] [data-cb="acceptAll"]', '[data-cookie-bar] [data-cb="declineAll"],[data-cookie-bar] [data-cb="saveNecessary"]', '[data-cookie-bar] [data-cb="declineAll"]', '[data-cookie-bar] [data-cb="saveNecessary"]', "plenty-shop-cookie=", "#pr-cookie-notice #btn-cookie-allow", ".pr-cookie-modal:not(._show) #pr-cookie-notice", "#pr-cookie-notice #btn-cookie-decline", "#pr-cookie-notice .pr-cookie-setting-btn", "#pr-cookie-setting-bar .pr-actions-buttons button.decline", "pr-cookie-consent=%5B%5D", "#pmc-pp-tou--notice", 'aside[aria-labelledby="pmc-privacy-banner-heading"]', 'aside[aria-labelledby="pmc-privacy-banner-heading"] button', "#js-consent-banner", "#js-consent-banner #js-cookie-reject-button", "#js-consent-banner #js-cookie-dismiss-button", "cookie-consent=", "#cookie-consent.c-cookie-banner", "#cookie-consent.c-cookie-banner .c-cookie-banner__quickform", "#cookie-consent .c-cookie-banner__quickform input[name='Purpose.Statistics'][value='False'] ~ input[type='submit']", "%22statistics%22%3Afalse", "#spopupCont", "#spopupCont #poper #cookie-btn.spopupbtnok", "#spopupCont #poper", ".cookiesBanner", ".cookiesBanner .wrapper-cookies", "#acb-banner", "#acb-banner #acb-accept-all-button", "#acb-banner #acb-deny-all-button", "#acb-banner input.acbCheckbox:checked:not([disabled])", "#acb-banner #acb-close-button", "PrestaShop-49722c247d445ea6ad5f50ec8153945a=0", ".gdpr-wrapper", ".gdpr-wrapper .gdpr-cookie-wrapper", "#cookie-bar .cb-enable,#cookie-bar .cb-disable,#cookie-bar .cb-policy", "#cookie-bar .cb-disable", "cb-enabled=accepted", ".azpp-cookie-consent", '"status":"reject_all"', '"status":"do_not_sell"', ".privacyArea", ".privacyArea .privacyTrigger.acceptAll", "#modulePrivacy .privacyTrigger.acceptCustom", ".privacyArea .privacyTrigger.rejectOptional", "#modulePrivacy input[type=checkbox]:checked", 'privacy={"necessary":"ok"}', ".c-cookie-banner-main.c-consent-window", ".c-cookie-banner .c-cookie-banner-main.c-consent-window", '.c-cookie-banner-main.c-consent-window[style*="visibility: visible"]', ".c-cookie-banner-main .c-cookie-banner-header-refuse-btn", 'elmo_vc={"purposes":["necessary"]', "#cookie-consent-banner #accept-button", "#cookie-consent-banner #deny-button", "#cookie-consent-banner #manage-settings-button", "#manage-cookies #save-button", "#pro_euclp_cookies_box", "#pro_euclp_cookies_box .pro_euclp_accept", "#euCookiesZone .european-union-cookies-agreement", "body[data-page-initialized=true]", "#euCookiesZone [data-testid=cookieBannerDismiss]", "EUCookieOff=t", "#pubtech-cmp", "#pubtech-cmp #pt-actions", "#pubtech-cmp #pt-close", '#pubtech-cmp a[href*="pubtech_action_reject_all"]', "#cookieConsentContainer", '#cookieConsentContainer [onclick^="purecookieDismiss"], script[src*="purecookie"]', "xpath///div[@id='qtip-growl-container']/div[contains(@class,'qtip-jgrowl')][.//div[contains(@class,'qtip-title')][contains(., 'ookie')]]", "xpath///div[@id='qtip-growl-container']/div[contains(@class,'qtip-jgrowl')][.//div[contains(@class,'qtip-title')][contains(., 'ookie')]]//a[contains(@class,'qtip-close')]", "#qc-cmp2-main,#qc-cmp2-container", "#qc-cmp2-container", "#qc-cmp2-ui,#qc-cmp2-usp", "#qc-cmp2-usp", "#qc-cmp2-usp .qc-cmp2-list-item-header,#qc-cmp2-usp .qc-cmp2-toggle-switch", "#qc-cmp2-usp .qc-cmp2-list-item:not(:has(.qc-cmp2-toggle-switch)) > .qc-cmp2-list-item-header", "#qc-cmp2-usp .qc-cmp2-toggle-switch", '#qc-cmp2-usp .qc-cmp2-toggle-switch > button[aria-checked="false"]', '#qc-cmp2-usp .qc-usp-ui-form-content button[mode="primary"]', "#qc-cmp2-mspa-success-modal button", "#disagree-btn", "#disagree-btn:not([disabled])", '.qc-cmp2-summary-buttons button[mode="secondary"]', '.qc-cmp2-summary-buttons button[mode="secondary"]:nth-of-type(2)', '.qc-cmp2-summary-buttons button[mode="secondary"]:nth-of-type(1)', "#qc-cmp2-ui .qc-cmp2-consent-info", '.qc-cmp2-toggle-switch > button[aria-checked="true"]', "#qc-cmp2-ui", ".qc-cmp2-buttons-desktop > button[mode=primary]", ".cookieconsent-popup:has(.ui-icon-wrench)", ".cookieconsent-popup .cookieconsent-button-wrap button:has(.ui-icon-wrench)", ".cookieconsent-edit-settings .cookieconsent-level-changer", ".cookieconsent-level-changer > a:nth-of-type(2).activated", ".cookieconsent-level-changer > a:first-of-type", ".cookieconsent-edit-settings .cookieconsent-dialog-close-btn", ".cookieconsent-edit-settings .cookieconsent-dialog-close", "cookie_consent_level=", "cookie_consent_level=tracking", "cookie_consent_level=targeting", "cookie_consent_level=functional", "#consent_iframe", "_svs=%7B%22v%22%3A1%2C%22c%22%3A%7B%221%22%3Atrue%2C%222%22%3Afalse%2C%223%22%3Afalse", "#r42CookieBar", "#r42CookieBar .cw-deny", "#r42CookieBar[open] .cw-deny", "#st_popup #st_popup_declineButton", "#st_popup", "feature-modular-cookiebanner-variant-one .cookiebanner-container", "#modular-cookiebanner-variant-one-consent-level-button-1", "feature-modular-cookiebanner-variant-one .cookiebanner-default-consent-button", "RABO_PSL=1", "[data-component=RADCookieOverlay] .CookieOverlay", "[data-component=RADCookieOverlay] .CookieOverlay_Buttons", "[data-component=RADCookieOverlay] .CookieOverlay_ButtonGroup > button:nth-child(2)", "cookie=False", "#js-legal-disclaimer.g-alert-cookie, #js-legal-disclaimer.cookie-consent-banner, #info-shell-legal-disclaimer, #oh-shell-legal-disclaimer", "#raksha-consent-banner", "#raksha-consent-overlay", "#raksha-decline", "#raksha-manage", "#raksha-btn-reject", "#cookie-policy-notification.r-sdk__policy-notification", "#cookie-policy-notification .r-sdk__button", "terms-notification=true", "#_cpmt-iframe", "#concentBanner", "#concentPopup", "#concentBanner .rdc-save-preferences, #concentPopup .rdc-save-preferences", "#concentBanner, #concentPopup", "#rcc-decline-button", "*:has(> * > #rcc-confirm-button):not(:has(#rcc-decline-button))", "#rcc-confirm-button", "*:has(> * > #rcc-confirm-button) button:not(#rcc-confirm-button)", "*:has(> * > #rcc-confirm-button)", '*:has(> * > * > #rcc-confirm-button)[style*="position: fixed"][style*="height: 100%"]', 'div[consent-skip-blocker="1"][id][data-bg]', 'div[consent-skip-blocker="1"][id][data-bg] > dialog > div > div > div > div > div > a[role=button][id]', '#cookie:has(a.remove_cookie[onclick*="/cookie/close"])', "xpath///div[@id='cookie']//a[contains(@class,'remove_cookie') and contains(@onclick,'/cookie/close') and (normalize-space(.)='OK' or normalize-space(.)='x' or normalize-space(.)='\xD7')]", '#cookie a.remove_cookie[onclick*="/cookie/close"]', "#cookie", '#notification-bar:has(> #close-notification[aria-label="Sluit de cookiebanner"])', '#notification-bar > #close-notification[aria-label="Sluit de cookiebanner"]', "#notification-bar", "#notification-bar > #close-notification", "cookie_notification_shown=true", ".consent-banner .consent--manage", ".consent-banner .consent-banner-copy", ".consent-modal .consent-bucket", ".consent-modal input[type=checkbox]:checked", ".consent-modal .consent-save", ".rws-app > .cookie-modal", "xpath///div[contains(@class, 'rws-app')]/div[contains(@class, 'cookie-modal')]//div[contains(@class, 'buttons')]//button[contains(normalize-space(.), 'n\xF6dv\xE4ndiga')]", "cookiesApproved=%5B%22REQUIRED%22%5D", "#xcm-bar.xcm-bar", "#xcm-bar.xcm-bar #xcm-btn", "#xcm-bar.xcm-bar:not(.xcm-bar-hide)", "#xcm-bar #xcm-btn", "#xcm-bar.xcm-bar-hide", "#mode-cookie-tip", "#mode-cookie-tip .cookie-tip-close", "cookie_tip_close=1", "#consent-wrapper", "#consent-wrapper > .button-container > button.close", "cookie_consent_status=", "dialog[aria-labelledby='cookie-dialog-title']:has(#deny-consent-button)", "dialog[aria-labelledby='cookie-dialog-title'] #deny-consent-button", "dialog[aria-labelledby='cookie-dialog-title'][open]", ".sv-cookie-management__banner", ".sv-cookie-management__banner.sv--visible", ".sv-cookie-management__banner .sv-manage-cookies", ".sv-cookie-management__modal--visible", "#functional-cookies:checked", "label[for='functional-cookies']", "#tracking-cookies:checked", "label[for='tracking-cookies']", ".sv-cookie-management__modal--visible .sv-save-close", "#scCookieBanner:has(.sc-cookie-reject)", "#scCookieBanner .sc-cookie-reject", "#scCookieBanner", "_consent=denied", '[data-module="ds-cookie-notification"]', '[data-module="ds-cookie-notification"] .js-accept-essential-cookies', "cookiePermissions=eyJuZWNlc3NhcnkiOnRydWUsInByZWZlcmVuY2VzIjpmYWxzZSwic3RhdGlzdGljcyI6ZmFsc2UsImNhbXBhaWducyI6ZmFsc2UsIm1hcmtldGluZyI6ZmFsc2V9", ".f-cmp-flyout-wrapper .f-gpc-banner", ".f-cmp-flyout-wrapper .f-gpc-banner .f-banner-close-btn", ".cc-window", ".f-gpc-banner .f-banner-close-btn", ".f-gpc-banner", "#gdpr-cookie", "#gdpr-cookie .js-allow-required", "gdpr-cookie=eyJwcmVmZXJlbmNlIjpmYWxzZSwic3RhdGlzdGljIjpmYWxzZSwibWFya2V0aW5nIjpmYWxzZX0", ".stpd_cmp_wrapper", ".stpd_cmp_wrapper .stpd_cmp_form", '.stpd_cmp_wrapper .stpd_flexed_btns button[type="submit"]', '.stpd_cmp_wrapper .stpd_flexed_btns button[type="button"]:not(.withdraw_consent)', ".stpd_cmp_wrapper .stpd_purposes_list", '.stpd_cmp_wrapper input[type="checkbox"]:checked', "euconsent-v2=", "#sg-cookiesBar #cookies-bar", "#sg-cookiesBar", "#sg-cookiesBar .btn-disable-all, #sg-cookiesBar .cookies-decline-all, #sg-cookiesBar #cookies-manage-save-modal", "#sg-cookiesBar #cookies-bar .btn-disable-all", "#sg-cookiesBar .cookies-decline-all", "#sg-cookiesBar input[type=checkbox][id$=-modal]:checked:not(:disabled)", "#sg-cookiesBar #cookies-manage-save-modal", "sg-cookies-", "-consent=1", ".consents-modal", ".consents-modal__footer consents-accept-necessary", "#InfoCookieTlo", "#CookieOknoUstawien", "#InfoCookie.DolnaInformacjaCookie, #InfoCookieIntegracje.DolnaInformacjaCookie", "#InfoCookieIntegracje", "xpath///div[@id='InfoCookie']//span[@id='InfoAkcept'][contains(., 'Nie pokazuj')]", "#InfoTylkoNiezbedneIntegracje", "#InfoUstawieniaIntegracje", "#CookieOknoUstawien input[type=checkbox]:checked:not(:disabled)", "#CookieOknoUstawien .GotoweCookieOkno span", "akceptCookie=tak", "#shopify-pc__banner", "#shopify-pc__banner__btn-decline", "#dux-privacy", '#dux-privacy [data-component-name="reject"]', '#dux-privacy:not([style*="opacity: 0"]) [data-component-name="reject"]', '#dux-privacy[style*="opacity: 0"]', ".cookie-permission-container[data-cookie-permission]", ".cookie-permission-container[data-cookie-permission] .js-cookie-configuration-button", ".cookie-permission-container .js-cookie-permission-button", ".cookie-permission-container .js-cookie-permission-button button", ".cookie-permission-container .js-cookie-configuration-button button", ".js-offcanvas-cookie-submit", "cookie-preference=1", "footer.pageFooter #cookieBanner.cookieBanner", "footer.pageFooter #cookieBanner.cookieBanner.visible", ".js-top-modal-window-container", ".js-top-modal-window-container .modal-link[href*='shueisha.co.jp/privacy']", ".js-top-modal-window-container .js-top-modal-content", ".js-top-modal-window-container button.close-modal.js-top-modal-close-button", ":is(#cookieBanner, .fixed_cookie_banner):has(a[href*='shueisha.co.jp/privacy'])", ":is(#cookieBanner, .fixed_cookie_banner):has(a[href*='shueisha.co.jp/privacy']) :is(#cookieBannerCloseButton, .cookieBannerCloseButton)", "sibbo-cmp-layout", "#rejectAllMain", "#silktide-banner", "#silktide-backdrop", "#silktide-banner .reject-all", "#silktide-banner .preferences", "#silktide-modal .preferences-reject-all", ".contentRender_name_plugins_common_cookie_banner", ".contentRender_name_plugins_common_gdpr_banner", ".contentRender_name_plugins_common_cookie_banner [data-sv-close-banner], .contentRender_name_plugins_common_gdpr_banner [data-sv-close-banner]", ".contentRender_name_plugins_common_cookie_banner > :has([data-sv-close-banner]), .contentRender_name_plugins_common_gdpr_banner > :has([data-sv-close-banner])", ".contentRender_name_plugins_common_cookie_banner, .contentRender_name_plugins_common_gdpr_banner", "#sd-cmp", "#sd-cmp [role='dialog']", "#sd-cmp [role='button'][title='Close']", '[data-t="site-notifications"] [data-notice-key="cookie_usage"]', ".SP-CookieUsageNotification", "xpath///button[contains(@class,'SP-CookieUsageNotification__ok')][normalize-space(.)='OK' or normalize-space(.)='Ok' or normalize-space(.)='Okay']", ".SP-CookieUsageNotification[data-sp-cookie-usage-notification-bootstrapped]", ".SP-CookieUsageNotification__ok", ".sv-cookie-consent-banner", '.sv-cookie-consent-banner form:has(input[name="necessary"]):not(:has(input[name="analytics"])) button[type="submit"]', '.sv-cookie-consent-banner button[type="button"]', '.sv-cookie-consent-banner button[type="submit"][form]', '.sv-cookie-consent-banner form input[type="checkbox"]:checked:not(:disabled)', ".sv-cookie-consent-modal", ".sv-cookie-consent-modal form:has(input[name=necessary]):not(:has(input[name=analytics])) button[type=submit]", ".sv-cookie-consent-modal .env-dialog__controls > button[type=button]", ".sv-cookie-consent-modal button[type=submit][form]", ".sv-cookie-consent-modal form input[type=checkbox]:checked:not(:disabled)", ".sv-iw-cookie #cookie-banner", ".sv-iw-cookie #cookie-banner button:nth-of-type(3)", ".sv-iw-cookie #cookie-banner button:nth-of-type(2)", ".sv-iw-cookie #cookie-banner button:first-of-type", ".sv-iw-cookie #cookie-modal", ".sv-iw-cookie #cookie-modal input[type=checkbox]:checked:not(:disabled)", ".sv-iw-cookie #cookie-modal > div > button:first-of-type", ".w-cookie", ".w-cookie-modal .w-cookie-actions button[name=accept]", ".w-cookie-modal .w-cookie-form", ".w-cookie-modal .w-cookie-option input[type=checkbox]:checked:not([disabled])", "cookie_consent=REQUIRED", ".snigel-cmp-framework", "#sn-b-custom", "#sn-b-save", "snconsent", "#cookie_modal", "#cookie_modal #id_cookies_necessary", "#cookie_modal #id_cookies_all", "#cookiebar-tra", "#cookiebar-tra #cb-RejectAll, #cookiebar #cb-RejectAll", "#cb-RejectAll", "cookieConsent=", ".sol-cookie-message", ".sol-cookie-message .sol-cookie-ok", ".cookies-banner .cookies-banner__button-group .cookies-banner__reject", ".cookies-banner .cookies-banner__reject", "_SC_analytics=reject", "#cookies_message", "#cookies-message-modal", "#cookies-close-deny, #cookies_frame > #cookies_close", "#cookies-close-deny", "#cookies_frame > #cookies_close", "cookies_message_bar_hidden=true", "cookies_google_targeting=false", "cookies_message_hidden=true", "body > div[id^='sp_message_container_']", ".cookiebar:has(.cookiebar-ko)", ".cookiebar .cookiebar-buttons .cookiebar-ko", "#opt-in .optin", "#opt-in .optin #optin--acceptAll", "#opt-in #optin--rejectAll, #opt-in #optin--onlyNeeded", ".cookie-banner-mount-point", ".cookie-banner-mount-point section[aria-label='Cookie banner'] :is(button.decline, button.manage)", ".cookie-banner-mount-point section[aria-label='Cookie banner']", ".cookie-banner-mount-point button.decline", ".cookie-banner-mount-point button.manage", ".cookie-banner-manager .manage-cookies-overlay", ".cookie-banner-manager .manage-cookies-overlay label.sqs-toggle--on input", ".cookie-banner-manager .manage-cookies-overlay button.save", ".cookie-banner-mount-point section[aria-label='Cookie banner'] button.accept", "#cookie-consent-banner #accept-cdp-cookie", "#reject-cdp-cookie", "squiz.cdp.consent", ".cookiepreferences_popup", "#rejectAllButton", "#_psaihm_main_div", "#_psaihm_overlay", "#_psaihm_main_div #_psaihm_id_accept_all_btn", "#_psaihm_refuse_all", "#_psaihm_button_param_accept", "#_psaihm_table_container input._psaihm_consentbox:checked", "#_psaihm_continue_without_accepting", "_psac_gdpr_consent_given=1", "_psac_gdpr_consent_purposes=;", "#GDPRConsentManagerContainer", "#GDPRConsentManagerContainer [data-atx-component='cookieConsent']", "#GDPRConsentManagerContainer #explicit_consent", "#GDPRConsentManagerContainer #ccmgt_explicit_preferences", "#GDPRConsentManagerContainer #ccmgt_preferences_reject", "CONSENTMGR=c1:0%7Cc2:0%7Cc3:0", ".js-cookie-banner.cookie-banner", ".js-cookie-banner.cookie-banner #cookie-banner__settings .js-cookie-settings__form", ".js-cookie-banner.cookie-banner .cookie-banner__actions .js-cookie-banner-settings-btn", "#cookie-banner__settings .js-cookie-settings__form", "#cookie-banner__settings .js-cookie-settings__checkbox:checked:not(:disabled)", "#cookie-banner__settings .cookie-settings__submit-btn", "-consent-necessary=1", "#controlled_banner-cookie_consent-consent_banner", "#controlled_banner-cookie_consent-privacy_setting_banner", "#controlled_banner-cookie_consent-consent_banner [data-search-key-button=reject_all_text]", "#controlled_submit-cookie_consent-consent_banner-privacy_setting_banner_link", "#controlled_banner-cookie_consent-privacy_setting_banner [data-search-key-button=reject_all_text]", "#controlled_banner-cookie_consent-privacy_setting_banner [role=checkbox][aria-checked=true]", "#controlled_banner-cookie_consent-privacy_setting_banner [data-search-key-button=save_settings_text]", '[data-testid="CookiesReminder"]', '[data-testid="CookiesReminder"] [data-testid="CookiesManage"]', "body:not(.disable-transition)", "dialog[open] .cookies-select-modal", "dialog[open] .cookies-select-modal .cookies-select-modal__buttons .ds-btn-apply-2-ds", " c=%7B%22essential", '.cc-window.cc-type-opt-change[aria-describedby="cookieconsent:desc"]', '.cc-window.cc-type-opt-change[aria-describedby="cookieconsent:desc"]:not(.cc-invisible)', ".cc-window.cc-type-opt-change .cc-deny", ".cc-window.cc-type-opt-change", "div[class*=cookieBanner].pencraft", "hideCookieBanner=", "#CookieConsentBannerPlaceholder", "#CookieConsentBannerPlaceholder .cookies-banner-container", "#consCookie.popup-overlay", "#consCookie.popup-overlay .cajaSetting", "#consCookie.popup-overlay .contenedorCookies", "#consCookie a.setting:not(.preferencias)", "#consCookie a.setting.preferencias", "#consCookie .cajaSetting input[type=checkbox]:checked:not(:disabled)", "CONS=100", "CONS=000", "#cookie-bar.cookie-bar", "#cookie-bar #cookie-settings__save-settings, #cookie-bar #cookie-bar__settings-open", "#cookie-bar:not(.cookie-bar--hidden)", "#cookie-bar__settings-open", "#cookie-settings #cookie-settings__save-settings", "input[name='swis-avg-allow-stats']:checked", "input[name='swis-avg-allow-tracking']:checked", "#cookie-settings__save-settings", "SWIS_AVG_ALLOW_STATS=false", "SWIS_AVG_ALLOW_TRACKING=false", ".js-consent-modal", ".js-consent-modal #validate_cookie_btn, .js-consent-modal #validate_cookie_foreign_btn", ".js-consent-modal .modal-content", ".js-consent-modal #validate_cookie_foreign_btn", ".js-consent-modal #unfold_cookie_btn", ".js-consent-modal .cookie-selection", ".js-consent-modal .cookie-selection input[type=checkbox]:checked:not(:disabled)", ".js-consent-modal #validate_cookie_btn", "_gtm_consent_prod=", ".syno_cookie_element", ".syno_cookie_element .btn_option", ".syno_cookie_element .scb_dialog_body", ".syno_cookie_element.scb_dialog input[type=checkbox]:checked:not(:disabled):not([readonly])", ".syno_cookie_element.scb_dialog .scb_btn_save", "syno_confirm_v5_answer", '"targeting":false', "#consent-banner-main", "#consent-banner-modal", '#consent-banner-modal a[href="#reject"]', '#consent-banner-modal a[href="#settings"]', "#consent-banner-settings", '#consent-banner-settings a[href="#reject"]', '#consent-banner-settings input[type="checkbox"]:checked:not(:disabled)', '#consent-banner-settings a[href="#save"]', "#consent-banner-modal, #consent-banner-settings", 'div[class^="cookies-banner-module_"]', 'div[class^="cookies-banner-module_cookie-banner_"]', 'div[class^="cookies-banner-module_small-cookie-banner_"]', "#cms-web-consent-modal", "#cms-web-consent-root #cms-web-consent-modal", "#cms-web-consent-modal [data-cy=consent-agree-necessary]", "tlnk_cwc=necessary_cookies=true", "marketing_cookies=true", "#tarteaucitronRoot", "#tarteaucitronAllDenied2", "#tarteaucitronCloseAlert", ".dsgvoaio-checkbox", ".dsgvoaio-checkbox > input:checked", "#tarteaucitronPersonalize", "#tarteaucitronAllDenied", "#tarteaucitronClosePanel", "#taunton-user-consent__overlay", "#taunton-user-consent__overlay:not([aria-hidden=true])", "#taunton-user-consent__toolbar input[type=checkbox]:checked", "#taunton-user-consent__toolbar button[type=submit]", "taunton_user_consent_submitted=true", "#tccCmpAlert", "xpath///span[contains(.,'Accetta solo cookie necessari')]", "#__tealiumGDPRecModal,#__tealiumGDPRcpPrefs,#__tealiumImplicitmodal,#consent-layer", "#__tealiumGDPRecModal *,#__tealiumGDPRcpPrefs *,#__tealiumImplicitmodal *", "#__tealiumGDPRecModal #rejectAll,#__tealiumGDPRcpPrefs #preferences_prompt_reject_all_submit", "#__tealiumGDPRecModal,#__tealiumGDPRcpPrefs,#__tealiumImplicitmodal", "#cm-acceptNone,.js-accept-essential-cookies,#continueWithoutAccepting,#no_consent,#consent_prompt_decline,#teal-decline-consent,#deny_full_submit_1", "#__tealiumGDPRecModal,#__tealiumGDPRcpPrefs > *", "div[data-cookie-consent]:has(> button[data-cookie-consent-close])", "div[data-cookie-consent] > button[data-cookie-consent-close]", "#termly-code-snippet-support", "#termly-code-snippet-support div", '[data-tid="banner-decline"]', '.t-preference-button,[data-testid="preferences-link"]', ".t-declineAllButton", ".t-preference-modal input[type=checkbox][checked]:not([disabled])", ".t-saveButton", ".termsfeed-com---nb", ".freeprivacypolicy-com---nb", ".iwt-com---nb", ".termsfeed-com---nb,.freeprivacypolicy-com---nb,.iwt-com---nb", ".cc-nb-reject", ".cc-nb-changep", 'input[cookie_consent_toggler="true"]', 'input[cookie_consent_toggler="true"]:checked', ".cc-cp-foot-save", ".cc_dialog.cc_css_reboot,.cc_overlay_lock", ".cc_dialog.cc_css_reboot", ".cc_dialog.cc_css_reboot button.cc_b_cp:last-of-type", ".cookie-consent-preferences-dialog .cc_cp_f_save button", "#reject-all", "#privacy-test-page-cmp-test", "#privacy-test-page-cmp-test-prehide", "#privacy-test-page-cmp-test-banner", "#tiGDPRecModal,#tiGDPRcpPrefs", "#tiGDPRecModal #consent_pref_center", "#tiGDPRecModal .privacy_prompt", "#consent_pref_center", "#tiGDPRcpPrefs .privacy_prompt", "#tiGDPRcpPrefs input.cookie-custom-checkbox:checked", "#preferences_prompt_submit", "c1:0%7Cc3:0%7Cc6:0%7Cc7:0", ".consent-banner-box", "consent-banner[component=consent-banner]", "button[data-consent=disagree]", "#cmpBanner", ".tna-cookie-banner", '.tna-cookie-banner button[value="reject"]', '.tna-cookie-banner__message--prompt button[value="reject"]', '.tna-cookie-banner__message--rejected button[value="close"]', "cookie_preferences_set=true", "%22usage%22%3Afalse", "#cookie-banner.visible", "#cookie-banner .cookie-banner__container [data-cookie-preferences-button]", "#cookie-banner.visible .cookie-banner__container", "#ensModalWrapper", "#cookie-banner.visible [data-cookie-preferences-button]", "#ensModalWrapper #ensSave", "#consentModalOverlay", "#ensModalWrapper[tabindex]", "#ensModalWrapper .ensCheckbox:checked:not(:disabled):not(#EssentialCookiesSlide)", "ENSIGHTEN_PRIVACY_TargetingCookies=0", ".tp-dialog.cookie-dialog", ".tp-cookie-banner", ".tp-dialog.cookie-dialog, :not(.tp-cookie-banner-tpl) > .tp-cookie-banner .show-cookie-dialog, #tp-cookie-banner > .tp-cookie-banner + .el-dialog__wrapper", ".tp-dialog.cookie-dialog button", ":not(.tp-cookie-banner-tpl) > .tp-cookie-banner .show-cookie-dialog", "#tp-cookie-banner > .tp-cookie-banner .tp-cookie-button-box", "#tp-cookie-banner > .tp-cookie-banner .tp-cookie-button-box .el-button--default", "#tp-cookie-banner > .el-dialog__wrapper", "#tp-cookie-banner > .el-dialog__wrapper .el-dialog__footer", "#tp-cookie-banner > .el-dialog__wrapper .el-dialog__footer button.is-plain:first-child", ".tp-dialog.cookie-dialog .tp-dialog-box", ".tp-dialog-box .tp-cookie-opt-out", ".tp-dialog-box .checkbox.clickable.checked", ".tp-dialog-box .tp-cookie-save", "_privacy_base=1", "_privacy_marketing=1", ".my-cookie-wrapper.cc-window", '.my-cookie-wrapper [aria-describedby="cookieconsent:desc"] .cc-reject-all', '.my-cookie-wrapper:not(.cc-invisible) [aria-describedby="cookieconsent:desc"] .cc-reject-all', "cookieconsent_dismissed1=reject_all", 'div.aem-page > div[class^="CookiesAlert_cookiesAlert__"]', "#transcend-consent-manager", 'dialog [data-testid="cookieBannerV2"]', '[data-testid="cookieBannerV2.rejectLink"]', "cookieConsentChoices=functional:true|analytics:false|marketing:false", '[class^="app_gdpr--"] button', '[class^="app_gdpr--"] div > button:first-of-type:nth-last-of-type(2)', '[class^="app_gdpr--"] div > button:nth-of-type(2):nth-last-of-type(2)', "tt_cons_addtl=", "pubConsent=", ".trillian-cookie-consent-banner", ".trillian-cookie-consent-banner .tpc2-reject-all", "t_privacy_confirmed=true", "#shopify-section-cookies-controller", "#shopify-section-cookies-controller #cookies-controller-main-pane", "#cookies-controller-main-pane a[data-tab-target=manage-cookies]", "#manage-cookies-pane.active", "#manage-cookies-pane.active input[type=checkbox][checked]:not([disabled])", "#manage-cookies-pane.active button[type=submit]", "#truendo_container .tru_cookie-dialog_wrapper", ".tru_cookie-dialog_accept-only", ".tru_cookie-dialog_accept-only .tru_cookie-dialog_message a[href='']", ".tru_cookie-dialog_wrapper [data-cy=banner-options-btn], .tru_cookie-dialog_wrapper [data-cy=optionsBtn], .tru_cookie-dialog_wrapper #tru_options_btn", "#truendo_container [data-cy=action-button-necessary]", '"adsPermitted":false', ".T360PurposeOverviewBanner", ".T360PurposeOverviewBanner :is(.T360_Banner_Reject, .T360Banner_Reject)", "_trust360_=", ".truste_popframe.trustarc_newcm_container", "cmapi_cookie_privacy=permit 1", "cmapi_cookie_privacy=permit 1,2", "#truyo-consent-module", "#truyo-cookieBarContent", "button#declineAllCookieButton", ".react-cookie-policy", ".react-cookie-policy #cookie-policy", "xpath///div[@class='react-cookie-policy']//button[contains(., 'Cookie settings')]", "button[role='switch']", "xpath///li[contains(., 'Tracking')]//button[@role='switch'][@aria-checked='true']", "xpath///li[contains(., 'Advertising')]//button[@role='switch'][@aria-checked='true']", "xpath///button[contains(., 'Save and close')]", "trybe_cookies_consent_tracking=false", "#cookie-onetrust-show-info", "#cookie-onetrust-only-necessary", "#onetrust-pc-sdk", "#onetrust-pc-sdk .ot-pc-refuse-all-handler", "#onetrust-pc-sdk input.category-switch-handler:checked", "#onetrust-pc-sdk .save-preference-btn-handler", "OptanonAlertBoxClosed", ".box-cookieterms", ".box-cookieterms[data-cookieterms-cookie]", "body:not(.cookieterms-hidden) .box-cookieterms", ".box-cookieterms [data-cookieterms-customize]", ".box-cookieterms-settings", ".box-cookieterms input[data-cookieterms-group]:checked:not([disabled])", ".box-cookieterms [data-cookieterms-save]", ".box-cookieterms [data-cookieterms-decline]", '"analytics":false', "iframe.tmblr-iframe--gdpr-banner", "[x-data^='cookieNotice('] > .fixed", "[x-data^='cookieNotice('][x-data*='tv2reg_cookie_consent']", "[x-data^='cookieNotice('] button[\\@click='acceptNecessary'], [x-data^='cookieNotice('] button[x-on\\:click='acceptNecessary']", '"handle":"marketing","value":false', "#twcc__mechanism", "#twcc__mechanism .twcc__notice", "#twcc__decline-button", "twCookieConsent=", '.cookie-notify:has(a[href*="cookies_policy"]) > button.btn, .cookies-policy-block:has(a[href*="cookies_policy"]) > button.btn, .header__cookie:has(a[href*="cookies_policy"]) > button.btn', "#js-cookieconsent", "#js-cookieconsent [data-cookieconsent-accept-none]", "CookieConsent=mandatory", "#cookieBanner #manage[data-bs-target='#cookieModalSettings']", "#cookieBanner .modal-content", "#cookieBanner #manage", "#cookieModalSettings form button[type=submit]", "#cookieModalSettings form input[type=checkbox]:checked", "#cookieModalSettings .modal-content", ".u-cookie-frame, .modal:has([data-attr='btn-reject-cookies'])", ".u-cookie-frame [data-attr='btn-reject-cookies'], .modal [data-attr='btn-reject-cookies']", ".u12-data-protection-notice", ".u12-data-protection-notice button", ".u12-data-protection-notice button.js-decline", "dialog.cookie-policy", "dialog.cookie-policy header", 'xpath///*[@id="modal"]/div/header', "dialog header", "button.js-manage", 'xpath///*[@id="cookie-policy-content"]/p[4]/button[2]', "dialog.cookie-policy .p-switch__input:checked", "dialog.cookie-policy .js-save-preferences", 'xpath///*[@id="modal"]/div/button', "_cookies_accepted=essential", "#politique_confidentialite", "#politique_confidentialite #cookies_settings", "#cookie_settings a.refuse", "UFT_cookie_notice_accepted=%5B%5D", "#catapult-cookie-bar", ".has-cookie-bar #catapult-cookie-bar", "catAccCookies", ".cookie-manager > #cookiebanner", "#p-cookie-banner", ".cookie-manager #cookiebanner #rejectall, #p-cookie-banner #p-cookie-banner__button--reject", ".cookie-manager > #cookiebanner, #p-cookie-banner", "uk-parliament.cookie-policy=eyJhbmFseXRpY3MiOmZhbHNl", ".cookie-notice-advanced", ".cookie-notice-advanced .cookie-bar #cookie-save", ".cookie-notice-advanced .cookie-bar", ".cookie-notice-advanced input:is(#social-cookies, #advertisement-cookies):checked", ".cookie-notice-advanced #cookie-save", "#avisoCookie.msgCookies", ".modal #chRendimiento", '#avisoCookie a[data-target^="#"], #avisoCookie a.modal-trigger', '.modal-footer button[onclick*="chRendimiento"]', ".modal #chRendimiento:checked", "aceptarCookies=true", "aceptarGoogleAnalytics=true", "#cookieControlWrapper", "#cookieControlWrapper #cookieControlBannerManage", "#cookieControlWrapper #cookieControlBanner", "#cookieControlBannerContent a[onclick*=unicityCookiesRejectAll]", "#cookieControlBannerContent a[onclick*=unicityCookiesManage]", "#cookieControlBannerManage", "#cookieControlBannerManage input[type=checkbox]:checked:not(:disabled)", "#cookieControlBannerManage a[onclick*=unicityCookiesSave]", "uni_cookies=0,0,0,0", "#cookieDisclaimer.cookieDisc", "#cpra_banner_wrapper", `#cookieDisclaimer a[onclick*="handleUSPrivacyAction('reject')"], #cpra_banner_wrapper #cpra_btn_reject, #userConsentForAdsPopup #allowButton, #gdprPopup #consent_dialog_allow`, "#cookieDisclaimer, #cpra_banner_wrapper, #userConsentForAdsPopup", "#gdprPopup-popup.ui-popup-active", "#cookieDisclaimer, #cpra_banner_wrapper", `#cookieDisclaimer a[onclick*="handleUSPrivacyAction('reject')"], #cpra_banner_wrapper #cpra_btn_reject`, "#userConsentForAdsPopup", '#gdprPopup a[data-rel="back"]', "usprivacy=1YYN", ".upcc-cookie-widget", ".upcc-cookie-widget .upcc-cookie-reject-all", ".upcc-cookie-widget .js-upcc-view-upcc-cookie-options", ".upcc-cookie-modal-container .upcc-cookie-reject-all", ".upcc-cookie-widget, .upcc-cookie-modal-container", "up-cookie-consent=true", 'up-cookie-consent-options=["strictly_necessary"]', "#usercentrics-root,#usercentrics-cmp-ui", "#usercentrics-cmp-ui", "#usercentrics-root", "#usercentrics-button", "#usercentrics-button #uc-btn-accept-banner", "#usercentrics-button #uc-btn-deny-banner", '#cookieconsent-window[aria-describedby="cookieconsent-desc"]', '#cookieconsent-window[aria-describedby="cookieconsent-desc"] #cookieconsent-dismiss', "#cookieconsent-window", "#cookie-banner[data-once~='vdg-cookies-block']", "#cookie-banner__config", "#cookie-banner__reject", "%22analytics%22:false", '#cookie-bar-modal[x-data^="initCookieBar"]', ".cookie-bar-wrapper:has(.allow-only-functional-cookie)", ':is([x-data^="initCookieBar"], .cookie-bar-wrapper) .allow-only-functional-cookie', ':is([x-data^="initCookieBar"], .cookie-bar-wrapper.active) .allow-only-functional-cookie', "#cookie_consent:has(#cookie_consent_details)", "#cookie_consent #cookie_consent_details :is(#functional-cookie-accept, .functional-button)", "#cookie_consent #cookie_consent_summary", "#cookie_consent_details :is(#functional-cookie-accept, .functional-button)", "cookieconsent_status=accept_functional", '[data-slot="dialog-content"] a[href*="vivenu.com/dataprivacy"]', '[data-slot="dialog-content"]:has(a[href*="vivenu.com/dataprivacy"]) button:nth-of-type(2)', ".consent-overlay", "div.consent[role=dialog]", "div.consent[role=dialog] .consent-main-buttons .consent-decline-all", "PrivacyPolicyOptOut", "UsageAnalysisConsent=yes", "#bottom-cookies", "#bottom-cookies.bottom-cookies-modal #openCookiesSettings", "#openCookiesSettings", "#cookiesModal #saveSelectedCookies", "#cookiesForm input[type=checkbox]:checked", ".cookie.cookie__floating", ".cookie.cookie__bar", ".cookie__floating__buttons__button--accept, .cookie__bar__buttons__button--accept", ".cookie.cookie__floating, .cookie.cookie__bar", ".cookie__floating__buttons__button--decline, .cookie__bar__buttons__button--decline", ".Cookie:has(> .Cookie__content + .Cookie__buttons)", ".Cookie > .Cookie__content + .Cookie__buttons", ".Cookie > .Cookie__buttons > button.Cookie__button--decline", "div[aria-labelledby=CookieAlertModalHeading]", "section[data-test=initial-waitrose-cookie-consent-banner]", "section[data-test=cookie-consent-modal]", "button[data-test=manage-cookies]", "button[data-test=submit]", "wtr_cookies_advertising=0", "wtr_cookies_analytics=0", ".warning.warning--active:has(.warning__apply)", ".warning:has(.warning__text) .warning__apply", "#cookie-notice:has(.btn-configure)", "#cookie-notice .btn-configure", "#modal_cookie-consent", "#modal_cookie-consent input.cookie-toggle:checked:not(:disabled)", ".cookie_preferences .modal-footer button.fill-primary", "%22ConsentAnalytical%22%3Afalse", "%22ConsentAdvertising%22%3Afalse", 'dialog#ltp-dialog, [id="id/ltp"]', 'iframe[id^="ngxFrame"][data-src]', 'dialog#ltp-dialog[open], [id="id/ltp/banner"]', "dialog#ltp-dialog", '[id="id/ltp"]', ".fs-cc-components,[fs-cc=banner]", 'script[src*="@finsweet/cookie-consent"]', ".fs-cc-components,[fs-cc=banner] [fs-cc=allow]", "[fs-cc=banner] [fs-cc=deny]", "[fs-cc=banner]", "fs-cc", "#wl-cc-box", "#wl-cc-backdrop", "#wl-cc-box #wl-cc-reject", "#wl-cc-cats:not(.wl-cc-exp)", "wl_cc_consent=%7B%22necessary%22%3Atrue%2C%22analytics%22%3Afalse%2C%22marketing%22%3Afalse", "#cookieNotice", "#cookieNotice #cookieNoticeDeclineCloser", "#cookieNotice #cookiesTracking:checked", "#cookieNotice #cookiesTracking", "#cookieNotice #loadExternRessources:checked", "#cookieNotice #loadExternRessources", "#cookieNoticeDeclineCloser", "allowTracking=0", ".ws_useragreement4alls", "[onclick^='ws_agreementcookies_set'], [onclick*='wsConsentDefault']", ":has(> .ws_useragreement4alls)", "[class*='ws_useragreementcgc_']:checked", "[class*='ws_useragreement_optin_']:checked", "[onclick*='wsConsentDefault']", "[onclick^='ws_agreementcookies_set']", "websale_useragreement=", ".wcc-overlay,.wcc-consent-container", ".wcc-consent-container", ".wcc-consent-container [data-tag=reject-button]", ".wcc-consent-container [data-tag=donotsell-button]", ".wcc-modal-open [data-tag=optout-option-toggle]", ".wcc-modal-open [data-tag=optout-option-toggle]:not(:checked)", ".wcc-modal-open [data-tag=optout-confirm-button]", ".wcc-consent-container [data-tag=settings-button]", ".wcc-modal-open [data-tag=detail-save-button],.wcc-consent-bar-expand [data-tag=detail-save-button]", ".wcc-modal-open [data-tag=detail-category-toggle] input:checked,.wcc-consent-bar-expand [data-tag=detail-category-toggle] input:checked", "action:yes", "[data-global-privacy-control-banner]", "[data-global-privacy-control-banner] > button", ".mw-cookiewarning-container", "[data-gtm-consent-modal]", "[data-gtm-consent-modal].gtm-consent__modal--show", "[data-gtm-consent-modal] [data-gtm-consent-close]", "[data-gtm-consent-modal] [data-gtm-consent-toggle]", "[data-gtm-consent-modal] [data-gtm-consent-details-section]", "[data-gtm-consent-modal] [data-gtm-consent-types-form] input[type=checkbox]:checked:not(:disabled)", "[data-gtm-consent-modal] [data-gtm-consent-submit]", '"analytics_storage":false', '"ad_storage":false', "div[role=dialog][aria-modal=true][aria-label*='WP']", "[data-comp-type=cookie-banner-root-wix],[data-hook=ccsu-banner-wrapper],[data-comp-type=cookie-banner-root-uou]", "[data-comp-type=cookie-banner-root-uou]", "[data-hook=consent-banner-decline-all-button]", "[data-hook=consent-banner-settings-button]", "[data-hook=consent-banner-settings-save-button]", "[data-hook=consent-banner-section-toggle-input]:checked:not(:disabled)", "[data-hook=ccsu-banner-decline-all]", "[data-hook=ccsu-banner-wrapper]", ".wccom-comp-privacy-banner .wccom-privacy-banner", ".wccom-privacy-banner__content-buttons button.is-style-secondary:has(+ button.is-style-secondary)", ".wccom-privacy-banner__content-buttons button.is-style-secondary:not([disabled]):has(+ button.is-style-secondary)", ".wccom-privacy-banner__content-buttons button.is-style-secondary:not([disabled]):has(+ button.is-style-primary)", ".wccom-modal__footer button.is-style-primary", ".wccom-customize-privacy-checkbox input[type=checkbox]:checked:not([disabled])", ".wccom-modal__footer button.is-style-primary:not([disabled])", "wccom_cookie_consent_analytics=denied", '[data-automation-id="legalNotice"]', '[data-automation-id="legalNoticeDeclineButton"]', "enablePrivacyTracking=false", "#cookieconsentpopup.lp__cookie_tooltip", ".privacy-widget:has(> button.accept-consent)", "#cookieconsentpopup.lp__cookie_tooltip, .privacy-widget > button.accept-consent", "#cookieconsentpopup.lp__cookie_tooltip, .privacy-widget:has(> button.accept-consent)", "#cookieconsentpopup .accept-consent, .privacy-widget > button.accept-consent", "consent_cookie=1", "#gdpr-cookie-consent-bar", "#gdpr-cookie-consent-bar #cookie_action_reject", "wpl_viewed_cookie=no", ".sp-dsgvo", ".sp-dsgvo.sp-dsgvo-popup-overlay", ".sp-dsgvo-popup-overlay .sp-dsgvo-privacy-btn-accept-nothing", ".sp-dsgvo-popup-overlay .sp-dsgvo-popup-close", "integrations%22%3A%5B%5D", ".wpcc-container", ".wpcc-container .wpcc-message", "#wpconsent-root", "#wpconsent-container", "wpconsent_preferences=", "#consent-container", "#consent_modal_backdrop", "#consent-container #consent-buttonConfirm", "#consent-container #consent-linkCustomConsent", "#consent-container #consent-buttonReject", "#consent-container #consent-buttonSaveChoices", "#consent-container input.consent-checkbox:checked:not(:disabled)", '#modal-privacy-settings button[data-grant="selected"]', '#modal-privacy-settings input[name="analytics"]:checked, #modal-privacy-settings input[name="ad"]:checked', "ad_storage=denied:analytics_storage=denied", ".notice--cookieAdvanced", ".js-notice.notice--cookieAdvanced #cookieConsentConfig", ".js-notice.notice--cookieAdvanced", '.notice--cookieAdvanced a[href*="reject=1"]', '.notice--cookieAdvanced [data-xf-click="toggle"][data-target="#cookieConsentConfig"]:not(.is-active)', '#cookieConsentConfig button[type="submit"]', '#cookieConsentConfig input[name^="consent["]:checked', "_consent=%5B%5D", ".cookie_warn", ".cookie_warn .cookie_warn_inner", "div[class^=cookie-consent-CookieConsent]", "#consent-settings-button", ".consent-banner-button-accept-overlay", "userConsent=%7B%22marketing%22%3Afalse", "#cookies-use-alert", ".disclaimer-opened #disclaimer-reject_cookies", ".disclaimer-opened #disclaimer-cookies", "#disclaimer-reject_cookies", "#yle-consent-sdk-container .ycd-banner", "#yle-consent-sdk-container button[name=accept-necessary-consents]", "tp-yt-iron-overlay-backdrop.opened", "ytd-consent-bump-v2-lightbox", "ytd-consent-bump-v2-lightbox tp-yt-paper-dialog", 'ytd-consent-bump-v2-lightbox tp-yt-paper-dialog a[href^="https://consent.youtube.com/"]', "ytd-consent-bump-v2-lightbox .eom-buttons .eom-button-row:first-child ytd-button-renderer:first-child #button,ytd-consent-bump-v2-lightbox .eom-buttons .eom-button-row:first-child ytd-button-renderer:first-child button", ".consent-bump-v2-lightbox", "ytm-consent-bump-v2-renderer", "ytm-consent-bump-v2-renderer .privacy-terms + .one-col-dialog-buttons c3-material-button:nth-child(2) button, ytm-consent-bump-v2-renderer .privacy-terms + .one-col-dialog-buttons ytm-button-renderer:nth-child(2) button", "#zdf-cmp-banner-sdk", "#zdf-cmp-main.zdf-cmp-show", "#zdf-cmp-main #zdf-cmp-deny-btn", ".zbottom-cp-container", ".zbottom-cp-container .zbc-cta-accept", ".zbottom-cp-container .zbc-cta-ignore button", ".zbottom-cp-container .zbc-manage-cta", ".zbc-main .zbc-cta-se", ".zbc-main .ctog-container.ctog-active:not(.ctog-no)", ".cookiewarning:has(> button.cookiewarning_yes.close-button)", ".cookiewarning > button.cookiewarning_yes.close-button", ".cookiewarning", ".c-cookies-notification-banner", ".c-cookies-notification-banner .c-notification-banner__close-button button", ".c-cookies-notification-banner .c-notification-banner--open", "_acceptCookies=true", ".cookie-alert-extended", ".cookie-alert-extended-modal", "a[data-controller='cookie-alert/extended/detail-link']", ".cookie-alert-configuration-input:checked", "button[data-controller='cookie-alert/extended/button/configuration']", "#piano-cookie_banner", "#piano-cookie_banner [data-close-button]", 'body > div#root > div#ccpa-iframe-theme-provider[data-testid="ccpa-iframe-theme-provider"] > div#ccpa-iframe[data-testid="ccpa-iframe"] > div#ccpa_consent_banner[data-testid="ccpa_consent_banner"] > div:not([id]) > div:nth-child(3):not([id]) > span:not([id]) > span:not([id]) > div:nth-child(2):not([id]) > span:nth-child(1):not([id]) > button#decline_cookies_button[data-testid="decline_cookies_button"]', "body:not([id]) > div#banner-0 > div:nth-child(1)#grouped-pageload-Banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#iubenda-cs-banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(5):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#ds-cookie-consent-banner > div:not([id]) > div:not([id]) > button:nth-child(4)#reject_optional_cookies", "body:not([id]) > div#banner-0 > div:nth-child(1)#grouped-pageload-Banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(3):not([id])", "body > div:not([id]) > div#clym-app-layout > div:nth-child(2)#clym-notice-layout > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div#clym-app-layout > div:nth-child(2)#clym-notice-layout > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div#clym-app-layout > div:nth-child(2)#clym-notice-layout > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1)#handle-reject-all", "body > div#banner-0 > div:nth-child(1)#grouped-pageload-Banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1)#dashboard > div:not([id]) > div:nth-child(2)#dashboard-body-container > div:nth-child(2):not([id]) > button:nth-child(4)#decline-text", "body > div:not([id]) > div:nth-child(4):not([id]) > button#cookie_all_reject", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#cookie_all_reject", "body > div:not([id]) > div#clym-app-layout > div:nth-child(2)#clym-notice-layout > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1)#handle-reject-all", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1)#dashboard > div:not([id]) > div:nth-child(1):not([id]) > button#decline-text", "body:not([id]) > div#root > div#ccpa-iframe-theme-provider > div#ccpa-iframe > div#ccpa_consent_banner > div:not([id]) > div:nth-child(3):not([id]) > span:not([id]) > span:not([id]) > div:nth-child(2):not([id]) > span:nth-child(1):not([id]) > button#decline_cookies_button", "body#moneyinvesting > div#top > div#content > div:nth-child(4):not([id]) > a:nth-child(11):not([id])", "body > div#banner-0 > div:nth-child(1)#grouped-pageload-Banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(3):not([id])", "body > div#app > div#app > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(1):not([id]) > div:not([id])", "body > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(4):not([id]) > div:not([id]) > a:nth-child(1)#ipoclick-btn-cookiebar-ik_ga_niet_akkoord_var_a", "body > div[id] > div > div > div > div > div:nth-child(3) > span > span > div:nth-child(2) > span:nth-child(1) > button", "#CybotCookiebotDialog", "#CybotCookiebotDialog #CybotCookiebotDialogBodyButtonDecline", "#CybotCookiebotDialogBodyButtonDecline", "marketing:false", "div[class^=CookieBannerContent__Container]", "button[class^=CookieBannerContent__Settings]", "div[class^=CookiePreferencesModal__CategoryContainer] input:checked", "div[class^=CookiePreferencesModal__ButtonContainer] > button", ".ecoPrivacyMain .ecoPrivacyButton", ".ecoPrivacyMain .ecoPrivacyButton:not([disabled])", "#app > div[role=alertdialog]", "#react-root div[role=alertdialog]", "#app > div[role=alertdialog] button .btn-text, #react-root div[role=alertdialog] button .btn-text", "#app > div[role=alertdialog], #react-root div[role=alertdialog]", "div[role=alertdialog] button:has(.btn-text) + button:has(.btn-text), div[role=alertdialog] > button:has(.btn-text)", "pref=%7B%22t%22%3A%22", "%22m%22%3Afalse%7D", "#overlay.cmp", "#overlay[role=dialog]", "#deny", "#edit-purpose-settings", "#save-purpose-settings", "ui_cid=OPTOUT", "#consentui #buttons .btn.amber", "#consentui #buttons .btn.red", "#consentui.screen_2 #filters", "#consentui #filters input[type=checkbox]:checked", "#consentui.screen_2 #buttons .btn.green", "#consentManagerButtons .entryPage .settings", "#consentManagerEntryPage", "#consentManagerButtons .entryPage .declineAll", "#consentManagerSettingsPage", "#consentManagerButtons .otherPages .declineAll", "#gdpr-consent-tool-wrapper", 'iframe[src^="https://cmp-consent-tool.privacymanager.io"]', "button#save", "#denyAll", ".okButton", "#manageSettings", ".purposes-overview-list", "button#saveAndExit", "span[role=checkbox][aria-checked=true]", "#cmp-app-container", "#save-all-pur", "#reminder", ".wd-cookie-banner.x-cookie-open", ".wd-cookie-banner", ".wd-cookie-banner.x-cookie-open .wd-cookie-banner__header__close", ".cookie-disclaimer", ".cookie-disclaimer a[href*='cprefs=change'], #myModal #cookieprefs", ".cookie-disclaimer, #myModal #cookieprefs", "#myModal #cookieprefs", "#cookieprefs input[type=checkbox]:checked", "#cookieprefs input[type=submit]", ".cookie-disclaimer a[href*='cprefs=change']", "adc-cookie-banner", "cookie_banner=closed", "[data-component=CookieBanner]", "[data-component=CookieBanner] [data-component=CookieBanner_AcceptAll]", "[data-component=CookieBanner] [data-component=CookieBanner_AcceptABCRequired]", "trackingconsent", ".fc-consent-root .fc-dialog-container", '[data-testid="cookies_reject"]', "#__next > div:has(> p > a[href*='cookie-policy'])", "xpath///div[@id='__next']/div[h2[normalize-space(.)='Cookies on this website']]", "xpath///div[@id='__next']/div[h2[normalize-space(.)='Cookies on this website']]//button[contains(., 'do not use analytics cookies')]", "#cookie-consent-banner #cookie-consent-accept", "#cookie-consent-accept", "cookie-consent=1", "aside#cookies,.overlay-cookies", "#cookies .cookies-btn", "#cookies #submitCookies", "#cookies #rejectCookies", "form.cookieprompt", "#cookiebar .cookies-settings, form.cookieprompt .cookies-settings", "form.cookieprompt .cookies-settings", "form.cookiepage .cookies-save", "form.cookiepage input.tracking-cookies:checked", "form.cookiepage input.tracking-cookies", "#cookiebar .cookies-settings", "#cookiebar .cookies-save", "#cookiebar input.tracking-cookies:checked", "#cookiebar input.tracking-cookies", "eyeo-ga-opt-out=true", ".overlay-footer", ".overlay-footer .btn-area-footer", ".fullpageCover", ".fullpageCover a[href*='/go/page/privacy.html']", ".fullpageCover div.rounded-full:nth-child(2)", "#popup_cookies", "#popup_cookies .boton_cookies", "#cookies_consent_div", "#popup_content.cookies_consent_div .config_cookies", "#popup_content #configurar_cookies", "#popup_content .cookie_button:not(.cookies_tecnicas) input[type=checkbox]:checked", "#popup_content .not_explanation.aceptar_cokies", "#cookiebar .cookiebar-confirm[data-accept=cookiebar]", "#cookie-overlay", "#decline-cookies", '[data-testid="main-cookies-banner-container"]', '[data-testid="main-cookies-banner-container"] section > div > div:nth-child(2) > div:nth-child(2) > button', "#aag-cookie-consent", "#cookieBarRGPDPop", "#cookieBarRGPDOverlay", "#cookieBarRGPDPop a[href*='loadParam']", "#cookieBarRGPDPopContent [data-cookiebarrgpd='0']", "#cookieBarRGPDPop a[href*='valideChoix']", 'cookieBarRGPDpublicite":0', "#gdpr-new-container", "#gdpr-new-container,#voyager-gdpr > div,#voyager-gdpr-2025 > div > div > button", "#voyager-gdpr > div", "#voyager-gdpr > div > div > button:nth-child(2)", "#voyager-gdpr-2025 > div > div > button", "#voyager-gdpr-2025 > div > div > button:nth-of-type(3)", "#voyager-gdpr-2025 > div > div > button:nth-of-type(2)", "#voyager-gdpr-2025 > div > div > button:last-of-type", "#voyager-gdpr-2025 > div > div > div > button:last-of-type", "#gdpr-new-container button:nth-of-type(3)", "#gdpr-new-container button:nth-of-type(2)", "#gdpr-new-container button:first-of-type", "#voyager-gdpr-2025", "#voyager-gdpr-2025 button:last-of-type", ".cookie-message", ".cookie-message .cookie-agree", "div:has(> div > button[allytmln=ally-cookie-consent])", "button[allytmln=ally-cookie-consent]", "#banner", "#banner #bannerFooter music-button", "#banner #bannerFooter", ".cookie-menu.js-cookie-banner", ".cookie-menu.js-cookie-banner .js-cookie-banner-close", "apaycnst-mktg=true", "#cookie-agreement", "#cookie-agreement .dismiss", "#a69-cookie-banner", "#a69-cookie-banner #a69-cookie-manage", "#a69-cookie-manage", "#a69-cookie-modal", "#a69-cookie-analytics:checked", "#a69-cookie-save", "a69_cookie_preferences=%7B%22analytics%22%3Afalse", "#arb-cookie-banner-section", "#arb-cookie-banner-section button", "#arb-cookie-banner-section button:first-of-type", "xpath///div[@role='dialog'][.//h2[normalize-space(.)='This website uses cookies']]", "xpath///div[@role='dialog'][.//h2[normalize-space(.)='This website uses cookies']]//button[normalize-space(.)='Manage Cookies']", "[role='dialog'] button[role='switch']#advertising", "[role='dialog'] button[role='switch'][aria-checked='true']", "xpath///div[@role='dialog'][.//button[@role='switch']]//button[normalize-space(.)='Save Preferences']", '"advertising":false', "#cookiePolicy", "#cookiePolicy #cookiebar-btn", "cookie policy accepted=", "#AcceptCookies2 #MrktConfirm", "#AcceptCookies", "#AcceptCookies input[onclick*=AcceptCookies2]", "#AcceptCookies2", "#AcceptCookies2 input[type=checkbox]:checked", "#AcceptCookies2 input[onclick*=SetCookie]", "MrktConfirm=false", "#footer-container ~ div", "#footer-container > div", "#gpc-banner-container", '"consent":"d"', "form.test-cookie-consent", 'form.test-cookie-consent [data-cookie-consent-target="screen1"]', 'form.test-cookie-consent [data-action="cookie-consent#showSettings"]', 'form.test-cookie-consent [data-cookie-consent-target="screen2"] [data-action="cookie-consent#rejectAll"]', "%22ad_storage%22%3Afalse", "body > div#shopify-section-popups > div:nth-child(1):not([id]) > modal-box#modal-popups-0 > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#cookieNotice > span:not([id])", "body > div#cookieAreaBase > div#cookieArea > p:nth-child(1):not([id]) > a:nth-child(3):not([id])", "body > div#cookieAreaBase > div#cookieArea > div:nth-child(2):not([id]) > a:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(4):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body#customcss > div#root > div:nth-child(4):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > span:nth-child(1):not([id]) > div:nth-child(5):not([id]) > button:nth-child(2):not([id])", "body > div#cookieConsentModal > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(2):not([id])", 'body > div#cookie-banner-deezer > div#gdpr-dir-tag > div:not([id])[data-testid="cookie-banner"] > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button#gdpr-btn-refuse-all[data-testid="gdpr-btn-refuse-all"]', "body:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div:not([id]) > div#gdpr_notification_container > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(6):not([id]) > div:not([id]) > a#gdpr_reject_button", "#cookiesPortletDiv", "#cookiesPortletDiv .cookie-buttons", "#cookiesPortletDiv button[aria-label='Only Mandatory']", "body > div#cookie-policy > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", ".MuiStack-root:has(a[href='/privacy']):has(button)", "body > div#portal-cookie-banner > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2)#portal-cookie-banner__wrapper > aside#portal-cookie-banner__content > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#messages > div:nth-child(1)#shopify-section-cookie-banner > section#shopify-section-cookie-banner > div:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div#messages > div:nth-child(2)#shopify-section-newsletter-banner > section:nth-child(1):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > form#trackingConsent > button:nth-child(2):not([id])", "body:not([id]) > div#cookieConsent > div:nth-child(2):not([id]) > a:nth-child(3):not([id])", "body > pnp-root:not([id]) > div:not([id]) > ui-storefront:not([id]) > footer:nth-child(8):not([id]) > cx-page-layout:not([id]) > cx-page-slot:not([id]) > cms-popia-component:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#___gatsby > div:nth-child(1)#gatsby-focus-wrapper > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button:not([id])", "body > div#layout-fea-0-bd-5-e-e-723-4027-aa-78-dbcc-846073-a-6 > div#page-62928 > div:not([id]) > div:nth-child(8)#\\31 e9783d2-3575-4c86-b6e8-bc0fd6b17307 > div#\\31 e9783d2-3575-4c86-b6e8-bc0fd6b17307-banner > div:nth-child(3):not([id]) > a:nth-child(1)#\\31 e9783d2-3575-4c86-b6e8-bc0fd6b17307-decline", "body > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div#c-cookiebanner > section:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > p:nth-child(1):not([id]) > button:not([id])", "body > div#wrapper > div:nth-child(4):not([id]) > button:nth-child(3):not([id])", "body > div#frame-modals > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#message-banner > div:not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div#cookiemodal > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:nth-child(5)#footer-cookie-buttons > a:nth-child(2)#footer-cookie-close", "body > div#notice-cookie-block > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div#termsfeed-pc1-notice-banner > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(2)#termsfeed_privacy_consent_banner_button_reject_all", "body > main:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > hathi-cookie-consent-banner:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cookies-modal > div:nth-child(2)#consent-modal > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id]) > span#consent-modal-refuse", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div#csm-wrapper > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#root > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(1):not([id]) > button:not([id])", "body > div#body-wrapper > section:nth-child(5):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div#wrapwrap > div:nth-child(5)#website_cookies_bar > div:not([id]) > div:not([id]) > div:not([id]) > section:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(1)#cookie-banner-essential", "body > aside:not([id]) > div:nth-child(2)#cookie-consent-modal > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div#cc--main > div#cc_div > div:nth-child(1)#cm > div#c-inr > div:nth-child(2)#c-bns > button:nth-child(2)#c-s-bn", "body > div#dk-cookie-message > div:not([id]) > button:nth-child(3):not([id])", "body > aside:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(3):not([id])", "body > div:not([id]) > div:not([id]) > div#ch2-settings-dialog > div:nth-child(3):not([id]) > div:nth-child(1)#ch2-settings > button:nth-child(4):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > a:nth-child(3):not([id])", "body:not([id]) > div#cookieConsentBanner > div#cookieConsentContent > button:nth-child(4)#closeConsentBanner", "body:not([id]) > div#cookies-modal > div:nth-child(2)#consent-modal > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id]) > span#consent-modal-refuse", "body > div:not([id]) > a:nth-child(3):not([id])", "body > div#root > div:nth-child(2):not([id]) > div:nth-child(5):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#tna-cookie-prompt-banner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#reject-cookie", 'body > div#ecom2-spa-root > div:nth-child(4)#layout-grid > div:nth-child(4)#main-content-v2 > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])[data-testid="cookie-btn-deny"]', "body > div:not([id]) > div:not([id]) > button:nth-child(5):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div#c-footerBrandV1 > footer:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div#ez-cookie-notification > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#ez-cookie-notification__decline", "body > div#site-wrapper > div:nth-child(1)#site-canvas > div#wrapper1 > main:nth-child(2)#mainSection1 > div:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2)#cookieadmin_reject_button", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:not([id])", "body > div#privacy-cookie-banners-root > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", 'body > div:not([id])[data-testid="CookieBanner"] > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])', "body > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > form:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div#oax-dialog-main > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > aside#cmp-banner > form:not([id]) > div:nth-child(4):not([id]) > section:not([id]) > div:not([id]) > button:nth-child(1)#cmp-deny-all", "body > main#main > div:nth-child(6):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > a:not([id])", 'body > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id])[data-testid="s-r-bu"]', "body > div#consent_manager-background > div:nth-child(1)#consent_manager-wrapper > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#consent_manager-accept-none", "body > div#ampsandConsentElement > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > section#cookie-consent > div:not([id]) > form:nth-child(3):not([id]) > div:nth-child(5):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div#cookie-banner > div:nth-child(2):not([id]) > button:nth-child(2)#cookie-banner-reject", "body > privacy-banner:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id]) > span:not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1)#hw-cc-notice-deny-btn", "body > div#cookieconsent > div#cookieconsent-bar > div:nth-child(3):not([id]) > a:nth-child(2)#decline", 'body > header#header > div:nth-child(1)#CookiesConsent > div:not([id]) > div:nth-child(2):not([id]) > form:nth-child(2):not([id]) > div:nth-child(5):not([id]) > button:not([id])[data-testid="AcceptRequiredCookies"]', "body > div:not([id]) > div:nth-child(1)#ccm-widget > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > dialog:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div#consent-manager > div:nth-child(1)#consent-banner > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button#consent-banner-btn-close", "body > section#cookie-policy > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#cbgccp-cookies-banner > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2)#cbg_ccp_cookie_refuse_optional_btn", "body > div > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(5):not([id]) > button:nth-child(1)#fd-unCheckAll", "body > div:not([id]) > div#csm-wrapper > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div#__next > div:nth-child(5):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#__next > div:nth-child(5):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > dialog:not([id]) > article:not([id]) > footer:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#entry > div:nth-child(2)#main > div:nth-child(4):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#__next > div:nth-child(3):not([id]) > dialog:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cb-cookie-warning > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#cb-cookie-warning__button--decline", "body > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#cookies-banner > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2)#gdpr-popup-v3-button-mandatory", 'body > div:not([id]) > div:nth-child(1):not([id])[data-testid="privacy-banner"] > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2)#react-aria491930105-\\:r29\\:[data-testid="privacy-banner-decline-all-btn-desktop"]', "body > div#tc-privacy-wrapper > div#footer_tc_privacy > div:not([id]) > div:nth-child(2)#footer_tc_privacy_container_button > button:nth-child(2)#footer_tc_privacy_button_2", "body > div#tc-privacy-wrapper > div:nth-child(2)#footer_tc_privacy > div:nth-child(2)#footer_tc_privacy_container_button > button:nth-child(3)#footer_tc_privacy_button_3", "body > div#app > div:nth-child(1)#__nuxt > div#__layout > div:not([id]) > div:nth-child(4):not([id]) > div:not([id]) > div:nth-child(4):not([id]) > a:nth-child(2):not([id])", "body > footer:not([id]) > div:nth-child(2):not([id]) > cookie-notice:nth-child(1):not([id]) > div#cookie-notice > div:not([id]) > div#cookie-notice-inner > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > button#cookie-notice-decline", "body > div#sell-root > div:nth-child(2)#appRoot > div:nth-child(7):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:nth-child(2)#BorlabsCookieBox > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > p:nth-child(6):not([id]) > a:not([id])", "body > div:not([id]) > div:nth-child(1)#consent-manager > footer:nth-child(2):not([id]) > button:nth-child(2):not([id])", 'body > div#app > div:nth-child(7):not([id])[data-testid="cookieBanner"] > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])[data-testid="onlyNecessaryCookies"]', "body > div#tc-privacy-wrapper > div#footer_tc_privacy > div:nth-child(2)#footer_tc_privacy_container_button > button:nth-child(2)#footer_tc_privacy_button_2", 'body > div#root > div:nth-child(2):not([id]) > div:not([id]) > div:not([id])[data-testid="cookie-consent-banner"] > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])', "body > div#c-pop > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(3)#ctl07_declinebtn", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(3):not([id])", "body > div#cookieNoticePro > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#cookieReject", "body > div#gdrp-cookieoverlay > div:nth-child(3)#cs2gdpr-cookiebanner > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button#btn-accept-required-banner", "body > div#seeGdprCookieConsent > div:not([id]) > p:nth-child(2):not([id]) > a:nth-child(2)#seeGdprReject", "body > div#tc-privacy-wrapper > div#popin_tc_privacy > div:nth-child(2)#popin_tc_privacy_container_button > button:nth-child(2)#popin_tc_privacy_button_2", "body > div#SgCookieOptin > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cookieman-modal > div:not([id]) > div:not([id]) > div:not([id]) > button:nth-child(6):not([id])", "body > div#c20343 > div:nth-child(1)#ikanos-privacy-cookielayer > div:nth-child(2):not([id]) > form:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div#consent-manager > div:nth-child(1)#consent-banner > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button#consent-banner-btn-close", "body > div#mm-2 > div:nth-child(2)#cookie-consent > div:not([id]) > form:nth-child(3):not([id]) > div:nth-child(5):not([id]) > button:nth-child(2):not([id])", "body > div#root > div:not([id]) > div:nth-child(2):not([id]) > main:nth-child(2):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body > div#application > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > a:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(6):not([id]) > div:nth-child(2)#BorlabsCookieBox > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > p:nth-child(5):not([id]) > a:not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > a:not([id])", "body > dialog#ai-hinweis > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(3):not([id]) > center:not([id]) > button:nth-child(1)#policy_notwendig", "body > div:not([id]) > div:nth-child(1)#ccm-widget > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(3):not([id])", "body > div#BannerRegion > div:nth-child(1)#Banner_cookie_0 > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2)#rejectAllBtn", "body > comply-consent-manager:not([id]) > div#comply-consent-manager > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", 'body > div#__next > div:nth-child(3):not([id]) > div:nth-child(3):not([id]) > div:nth-child(3):not([id])[data-testid="cookie_notice_reject_all_button"]', "body > div#tc-privacy-wrapper > div:nth-child(2)#popin_tc_privacy > div:nth-child(2)#popin_tc_privacy_container_button > button:nth-child(2)#popin_tc_privacy_button_2", "body > div#__nuxt > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > section:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div#cdk-overlay-0 > mat-dialog-container:nth-child(2)#mat-mdc-dialog-0 > div:not([id]) > div:not([id]) > hra-consent-layer-ui:not([id]) > hra-cookie-buttons:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id]) > span:nth-child(2):not([id])", "body > div#cookie-note-main > div:nth-child(2):not([id]) > button:nth-child(2)#cookie-accept-required", "body > aside:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(5):not([id]) > button:nth-child(4):not([id])", "body > div#cookiebanner-body > div:nth-child(1)#cookiebanner > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:not([id])", "body > div#app > div:nth-child(1)#header > div:nth-child(2)#cookie_banner > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#__nuxt > div#__layout > div#app > div:nth-child(6):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div#__nuxt > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > section:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(1):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > section#shopify-pc__banner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(3)#shopify-pc__banner__btn-decline", "body > div#__nuxt > div#__layout > div:not([id]) > section:not([id]) > div:nth-child(2):not([id]) > section:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#consent-manager > div:nth-child(1)#consent-banner > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button#consent-banner-btn-close", "body > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > p:nth-child(2):not([id]) > a:not([id])", "body > div#tc-privacy-wrapper > div:nth-child(2)#popin_tc_privacy > div:nth-child(2)#popin_tc_privacy_container_button > button:nth-child(3)#popin_tc_privacy_button_3", "body > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body > div#modal-wrap > section#modal-content > div:not([id]) > div:nth-child(2)#cookie-banner > div:not([id]) > div:nth-child(1)#s1 > p:nth-child(5):not([id]) > a:nth-child(4)#button_reject", "body > div#cookie-consent-banner > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3)#btn-reject-all", "body > div#cc-button > div#cc-text > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#page > main:nth-child(2)#index > div:nth-child(3)#cookie-settings-modal > div:not([id]) > div#cookie-settings > div:nth-child(4):not([id]) > button:nth-child(3):not([id])", "body > div#__nuxt > div:nth-child(8)#cookie-banner > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", 'body > div#consentBanner > div:not([id]) > div#gdpr-banner-container[data-testid="gdpr-banner-container"] > dialog#gdpr-banner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(3)#gdpr-banner-cmp-button[data-testid="gdpr-banner-decline-button"]', "body > div#consentWidget > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#denyBtn", "body > div#portal-footer > div:nth-child(3):not([id]) > div:nth-child(2)#footer-analytics > div#CookieConsent > div:not([id]) > p:nth-child(2):not([id]) > span:nth-child(3):not([id]) > button:not([id])", "body > div#cookieman-modal > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "#tc-privacy-wrapper", '[data-testid="consentManager"]', '[data-testid="privacyBanner-rejectAll"]', "body > div#ww_bzga_matomo_cookiebanner > div:not([id]) > p:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#ddt-M1 > div:not([id]) > div:nth-child(1)#ddt-Seite1 > div:nth-child(2)#ddt-sectionFirst > p:nth-child(4):not([id]) > a:nth-child(2):not([id])", "body > div:not([id]) > div#cookies-bar > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div#csm-wrapper > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body > div#cookie-consent > div:not([id]) > form:nth-child(3):not([id]) > div:nth-child(5):not([id]) > button:nth-child(2):not([id])", 'body > div:not([id]) > div:nth-child(2)#cookie-consent[data-testid="cookie-consent-banner"] > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])', "body > div#tc-privacy-wrapper > div#popin_tc_privacy > div:nth-child(2)#popin_tc_privacy_container_button > button:nth-child(3)#popin_tc_privacy_button_3", "body > div#_evidon-barrier-wrapper > div:nth-child(2)#_evidon-banner > div:nth-child(1)#_evidon-banner-content > div:nth-child(5):not([id]) > button:nth-child(2)#_evidon-barrier-declinebutton", "div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2)#BorlabsCookieBox > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > p:nth-child(2):not([id]) > a:not([id])", 'body > div#consent_modal[data-testid="Over18ModalVariant1Modal"] > div:nth-child(2):not([id])[data-testid="Over18ModalVariant1Content"] > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1)#reject-all-cookies-btn', "body > div:not([id]) > div:nth-child(1)#matomoCookieNotification > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button#disallowMatomoCookieNotification", "body > div:not([id]) > aside:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > span:nth-child(1):not([id]) > button:not([id])", "body > div#__nuxt > div:not([id]) > div:nth-child(6):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(1):not([id])", "body > div:not([id]) > div:nth-child(1)#cookies-eu-banner > div:not([id]) > div:not([id]) > button:nth-child(4)#cookies-eu-reject", "body > div:not([id]) > div:not([id]) > div:not([id]) > ul:not([id]) > li:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#__nuxt > div:not([id]) > div:nth-child(8):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div#optInId > div:nth-child(4):not([id]) > button:nth-child(2)#Ablehnen", "body > div:not([id]) > div#cpnb > div#w357_cpnb_outer > div:not([id]) > div:nth-child(2):not([id]) > span:nth-child(2)#cpnb-decline-btn", "body > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#a-page > div:nth-child(5)#sc-content-container > div:nth-child(1):not([id]) > div:nth-child(12):not([id]) > div:nth-child(1) > div:nth-child(2)#cookie-consent-window > div:not([id]) > div:nth-child(2) > div:nth-child(1)#cookie-consent-continue > div:not([id]) > a:not([id])", "body > div:not([id]) > div:nth-child(1)#ccm-widget > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#__next > div:not([id]) > div:not([id]) > main:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(4):not([id]) > div:nth-child(1):not([id])", "body > div#acris--page-wrap--cookie-permission > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#cookie-permission--accept-only-functional-button", "body > div#cookieConsentBanner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#consentMinimal", "body > div#elGuestTerms > div:not([id]) > div:nth-child(2):not([id]) > form:not([id]) > button:nth-child(3):not([id])", "body > div#mm-0 > div:nth-child(2)#cookie-consent > div:not([id]) > form:nth-child(3):not([id]) > div:nth-child(5):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id])", "body > div#cookiebar > div:not([id]) > span:nth-child(3)#declineCookie > button:not([id])", "body > main#content > aside:nth-child(10):not([id]) > div:not([id]) > wm-stack:nth-child(2):not([id]) > wm-button:nth-child(1):not([id]) > button:not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button:not([id])", "body > div#tc-privacy-wrapper > div#footer_tc_privacy > div:nth-child(1)#footer_tc_privacy_container_text > div#footer_tc_privacy_text > h2:nth-child(1):not([id]) > button#footer_tc_privacy_button_2", "body > div#bandeau_cgv > div:not([id]) > div:not([id]) > a:nth-child(1)#dismiss-cookies", "body > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > section:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > ul:nth-child(2):not([id]) > li:nth-child(2):not([id]) > button:not([id])", "body > article:not([id]) > header:nth-child(2)#header > div:nth-child(1)#cookie-modal > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div#root > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button:not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#cookieRefuse", "body > div#cookie-banner > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(3)#reject-cookies", "body > div:not([id]) > div:not([id]) > div:not([id]) > div#P0-0 > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > ul:nth-child(2):not([id]) > li:nth-child(2):not([id]) > button:not([id])", "body > div#ppms_cm_consent_popup_7a8e4f13-52de-496c-9239-96129f02cf08 > div#ppms_cm_popup_overlay > div#ppms_cm_popup_wrapper > div:nth-child(1)#ppms_cm_popup > div:nth-child(3)#ppms_cm_popup_main_id > div:nth-child(2)#ppms-124c44a9-52c6-4136-9964-9a6be955271a > button:nth-child(2)#ppms_cm_disagree", "body > div#cookie > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > a:nth-child(2):not([id]) > span:not([id]) > span:not([id])", "body > div#cookie-message > a:nth-child(2):not([id])", "body > div#tc-privacy-wrapper > div#popin_tc_privacy > div:nth-child(1)#popin_tc_privacy_container_text > div#popin_tc_privacy_text > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#refuse_all", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#main > div:nth-child(11)#cookbar_overlay > div#cookbar > span:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > a:nth-child(3):not([id])", "body > div#tc-privacy-wrapper > div#popin_tc_privacy > div:nth-child(2)#popin_tc_privacy_container_button > button:nth-child(1)#popin_tc_privacy_button", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > main#main > dialog:nth-child(25):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > div#cookie-banner > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:not([id])", 'body > div:not([id]) > div:nth-child(1):not([id])[data-testid="privacy-banner"] > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2)#react-aria526766221-\\:r0\\:[data-testid="privacy-banner-decline-all-btn-desktop"]', "body > div:not([id]) > form:nth-child(4):not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button#cookie_consent_use_only_functional_cookies", "body > div#orejime > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > ul:nth-child(2):not([id]) > li:nth-child(2):not([id]) > button:not([id])", "body > app-root:not([id]) > div:not([id]) > app-cookie-banner:nth-child(3):not([id]) > div:nth-child(1):not([id]) > div:nth-child(2)#reject-cookie-link > button#reject-cookie", "body > main:not([id]) > footer:nth-child(4)#footer > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3)#acb-banner > div:nth-child(2)#acb-action > button:nth-child(1)#acb-deny-all-button", "body > div#st-cmp-v2 > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > span:not([id]) > div:not([id])", "body > div#cookie-consent > div:nth-child(1):not([id]) > button:nth-child(1)#cookie-deny-button", "body > div#advencyRgpd > button:nth-child(3)#refuseAll", "body > div#rgpd > div:nth-child(1):not([id]) > div:not([id]) > button:nth-child(1):not([id])", "body > div#consent > user-consent:nth-child(1):not([id]) > consent-dialog:not([id]) > consent-content:not([id]) > consent-message:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(2):not([id])", "body > div#bottom-banner > div:nth-child(3):not([id]) > div:nth-child(1)#cookies-win > button:nth-child(2):not([id])", 'body > div:not([id]) > div:not([id])[data-testid="cookie-consent-banner"] > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])', "body > div:not([id]) > div:nth-child(2)#BorlabsCookieBox > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > p:nth-child(8):not([id]) > a:not([id])", "body > div#___gatsby > div:nth-child(1)#gatsby-focus-wrapper > div:nth-child(3):not([id]) > div:nth-child(3):not([id]) > button:nth-child(1):not([id])", "body > div#privacy-banner > div:nth-child(3):not([id]) > button:nth-child(1)#privacy-rejected", "body > div#cookie > div:not([id]) > div:nth-child(2)#Buttondivanalytic > button:nth-child(2):not([id])", "body > div:not([id]) > p:nth-child(2):not([id]) > button:nth-child(2)#seopress-user-consent-close", "body#top > div:not([id]) > form:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#cookie-notice > div:not([id]) > span:nth-child(2)#cn-notice-buttons > button:nth-child(2)#cn-refuse-cookie", "body:not([id]) > div#BannerRegion > div:nth-child(1)#Banner_cookie_0 > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2)#rejectAllBtn", "body:not([id]) > div#tc-privacy-wrapper > div#footer_tc_privacy > div:nth-child(2)#footer_tc_privacy_container_button > button:nth-child(2)#footer_tc_privacy_button_2", "body#html-body > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body:not([id]) > div#modal-root > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id]) > div:not([id])", "body:not([id]) > div#root > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cookie-banner-container > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#cookie-law > div:not([id]) > div:nth-child(1):not([id]) > button:nth-child(12):not([id])", "body:not([id]) > div#__next > div:not([id]) > div:nth-child(8):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body:not([id]) > header#header > div:nth-child(1)#CookiesConsent > div:not([id]) > div:nth-child(2):not([id]) > form:nth-child(2):not([id]) > div:nth-child(5):not([id]) > button:not([id])", "body:not([id]) > div#__next > div:not([id]) > div:nth-child(6)#cw-footer-container > footer:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button:not([id]) > span:nth-child(2):not([id]) > span:nth-child(1):not([id])", "body > div#cookie_alert > div:nth-child(2)#cookie_alert_container > div#cookie_alert_text > div:nth-child(2):not([id]) > div:nth-child(2)#js-cookie_alert_button_decline > a#cookie_alert_decline", "body > div > div:not([id]) > div:nth-child(1) > div:nth-child(2)#cookieDisclaimer > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:not([id])", "body:not([id]) > dialog:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body:not([id]) > dialog:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body#unlogged-body > div#cookie-banner-deezer > div#gdpr-dir-tag > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button#gdpr-btn-refuse-all", "body:not([id]) > section:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > section#cookie-policy > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body:not([id]) > div#cc--main > div#cc_div > div:nth-child(1)#cm > div#c-inr > div:nth-child(2)#c-bns > button:nth-child(1)#c-s-bn", "body > div#headlessui-portal-root > div:not([id]) > div:nth-child(2):not([id]) > div#headlessui-dialog-_r_0_ > div:nth-child(2):not([id]) > div#headlessui-dialog-panel-_r_6_ > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > a:nth-child(3):not([id])", "body:not([id]) > div#cookie-wall > div#cwi > div:nth-child(4)#cw-controls > div:not([id]) > div:nth-child(4):not([id]) > button:nth-child(2)#cwc-reject > span:not([id])", "body > div:not([id]) > div#csm-wrapper > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#sliding-popup > div#cookiepopup > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div#elGuestTerms > div:not([id]) > div:nth-child(2):not([id]) > form:not([id]) > button:nth-child(3):not([id])", "body:not([id]) > div#consent-box > p:not([id]) > span:not([id]) > button:nth-child(2)#decline-cookies", "body > div#__next > div:nth-child(3)#cookieBanner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > form:nth-child(2):not([id]) > button:nth-child(2)#cookieConsentRejectAll", "body:not([id]) > section:not([id]).privacy-banner > div:not([id]).privacy-banner__wrap > div:not([id]).privacy-content > div:not([id]).privacy-banner__grid-wrap > div:not([id]).privacy-banner__grid-col.privacy-banner__inner > div:nth-child(3):not([id]).privacy-banner__actions.privacy-banner__set.privacy-banner__btn-wrapper--stacked > button:nth-child(2):not([id]).privacy-banner__btn.privacy-banner__btn--secondary.privacy-banner__reject", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button#confirmSelection", "body:not([id]) > div:not([id]) > div:nth-child(5):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2)#gdpr-popup-v3-button-mandatory", "body#hgcBody > div#cookies-banner > a:nth-child(6)#btnCookieBannerNo", "body:not([id]) > dialog#biccy-banner > div#biccy-prompt > div:nth-child(2):not([id]) > button:nth-child(2)#biccy-reject-button", "body:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div > div:nth-child(1) > div:nth-child(3) > div:nth-child(2)", "body > div > div:nth-child(1) > div:nth-child(1) > div:nth-child(2) > button:nth-child(3)", "body > aside#cookies-policy > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > form:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(14):not([id]) > div#m-cookienotice > div:not([id]) > div:nth-child(3)#action-custom-css > a:nth-child(2):not([id])", "body > div#__nuxt > div#__layout > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div#__nuxt > div#__layout > div:not([id]) > div:nth-child(4):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "[data-testid='reject-button']", "body > div > div:nth-child(10) > div:nth-child(2) > div > div:nth-child(2) > div:nth-child(2)", "body > div#shopui-cookie-popup-container > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > a:not([id])", "body > section:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body:not([id]) > div#__nuxt > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cookies_overlay > div#cookies_banner > div:nth-child(2)#cookies_decision > button:nth-child(2)#cookies_read_declined", "body:not([id]) > div#__next > div:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div#cookie-consent-popup > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#cookie-reject", "body > div#cookie-consent-banner > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2)#btn-reject-all", "body > div:not([id]) > div:nth-child(78)#cookie-banner > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cribbon > div:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2)#select-only-necessary > span:not([id])", "body > div#app > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#modal-cookiesettings > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(3):not([id])", "body > div:not([id]) > div:nth-child(1)#react-content > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id]) > span:not([id])", "body > div:not([id]) > div#cookieconsent > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > div:not([id]) > button:nth-child(1)#cookie-deny", 'body > div#__next > div:nth-child(2)#cookieBanner > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])[data-testid="refuse-all-cookies"]', "body > aside#cookie-consent-banner > div:not([id]) > form:not([id]) > div:nth-child(2):not([id]) > fieldset:nth-child(3):not([id]) > ul:not([id]) > li:nth-child(1):not([id]) > button:not([id])", "body > div#cookie-popup > div:nth-child(2):not([id]) > button:nth-child(2)#reject-cookies", "body > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", 'div[data-test-consent-banner-popup] button[aria-label="Niet accepteren"]', "body > dialog:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > p:nth-child(2):not([id]) > a:nth-child(1):not([id])", "body > div:not([id]) > div:nth-child(1)#react-content > div:nth-child(8):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div#consent-modal > div:not([id]) > div:nth-child(1)#consent-intro > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button#consent-deny-all", "body > div:not([id]) > div:nth-child(5):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#cookielaw_banner > div:nth-child(2):not([id]) > a:nth-child(2)#cookielaw_reject", "body > div#__next > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(8):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#_app > div:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#radix-_r_0_ > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(3):not([id])", "body > div#nimbu-consent > div:not([id]) > div:not([id]) > div:not([id]) > p:nth-child(2):not([id]) > button:nth-child(2):not([id])", '.ind-cbar[data-testid="ind-cbar"]', '.ind-cbar[data-testid="ind-cbar"] button[save-cookie][data-value="0"]', "body > div:not([id]) > div#container > section:nth-child(1):not([id]) > div:not([id]) > div#block-mumc-info-cookiesui > div#cookiesjsr > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > web-overlay:not([id]) > web-cookie-consent:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#wpca-lay-out-wrapper > div:nth-child(1)#wpca-bar > div:nth-child(2)#wpca-bar-meta > button:nth-child(3):not([id])", 'body > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#cookiebanner-decline[data-testid="button-cookie-decline"]', "body > div#cookiebanner > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > span:nth-child(2):not([id]) > a:nth-child(2)#cookie-niet-akkoord", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > section#cookie-consent > div:nth-child(7):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1)#button-\xABr0\xBB", "body > aside#cookies-policy > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:nth-child(3):not([id]) > form:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(4):not([id]) > div:not([id]) > a:nth-child(4):not([id])", "body > div#cookies-eu-wrapper > div#cookies-eu-banner > div:nth-child(2)#cookies-eu-buttons > button:nth-child(1)#cookies-eu-reject", "body > div#wrapper > footer:nth-child(3):not([id]) > div#footer > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2):not([id])", "body > div#cc-card > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > a:nth-child(2)#cc-dismiss-btn", "body > div#__nuxt > div:not([id]) > div:nth-child(3):not([id]) > section#modal > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#cookie-bar > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id]) > span:not([id])", "body > div#CNID_4eb59999-7a35-450d-a08f-1661b81e5bc4 > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#refuseAll", 'body > div:not([id]) > div:nth-child(3)#cookie-consent[data-testid="cookie-consent-banner"] > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])', "body > div#_rootSiteLayout > aside:nth-child(5):not([id]) > div:nth-child(3):not([id]) > aside:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(3):not([id])", "body > footer:not([id]) > div:nth-child(2)#cookieParent > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1)#noCookies", "body > div#siteWrapper > div:nth-child(1):not([id]) > section:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(4):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(1):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2)#bcSubmitConsentToNone", "body > aside:not([id]) > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(3):not([id])", "body > div:not([id]) > div:not([id]) > button:nth-child(4):not([id])", "body > shn-dialog#firstTimeVisitorCookieDialog > dialog:not([id]) > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > section#cc-window-overlay > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > button:nth-child(1):not([id])", "body > div#cookieconsent > section:not([id]) > div:not([id]) > p:nth-child(6):not([id]) > a:not([id])", "body > ticketswap-portal:not([id]) > ul:not([id]) > div:not([id]) > div:not([id]) > span:nth-child(2):not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2)#ez-cookie-notification > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2)#ez-cookie-notification__decline", "body > div#__nuxt > div:nth-child(2)#__layout > div:not([id]) > div:nth-child(3):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2)#cookie-consent > div:not([id]) > dialog:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > div#cookie-consent-modal-description > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(2):not([id]) > span:not([id])", "body > div:not([id]) > div:nth-child(3):not([id]) > div:not([id]) > section#chakra-modal-\\:rd\\: > div#chakra-modal--body-\\:rd\\: > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > form:not([id]) > button:nth-child(2)#cookie_settings_disallowed", "body > div#app > div:not([id]) > div:nth-child(10):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(1):not([id]) > div:nth-child(1)#cookie-banner > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(3)#cookie-banner-reject", "body > main#maincontent > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#svid10_17c33ae315e94c3ae6647bf > div:nth-child(1)#svid12_3f29801717d5618f8df9bf4 > div:nth-child(2):not([id]) > div:not([id]) > button:nth-child(4)#afCookieDecline", "body > aza-app:not([id]) > aza-shell:not([id]) > div:nth-child(2):not([id]) > aza-cookie-message:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id]) > span:not([id])", "body > div#app > div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#geotargetlygeoconsent1749551372435container > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#ppms_cm_consent_popup_d0973bf6-9332-4ce3-bc7c-09678b958daf > div#ppms_cm_popup_overlay > div:nth-child(2)#ppms_cm_popup_wrapper > div#ppms_cm_popup > div#ppms_cm_popup_main_id > div#ppms_cm_popup_responsive_wrapper_id > div:nth-child(2)#ppms-4ad6f5ba-7508-4caa-bd23-e8e9a4bd64b2 > div:nth-child(2)#ppms-85b001a4-b886-4f7e-82d5-bd34f50f3266 > button:nth-child(2)#ppms_cm_reject-all", "body > div#__nuxt > div:nth-child(2)#main-wrapper > div:nth-child(5):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#shopify-section-sections--20055816765655__privacy-banner > cookie-bar:not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > form:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#cookie-consent-modal > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(4):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div#radix-_r_2_ > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > section#component-cookie-banner > div:nth-child(1):not([id]) > footer:nth-child(4):not([id]) > div:nth-child(1):not([id]) > button:nth-child(2)#cookie-banner-accept-essentials", "body > form#cookieConsentForm > div#cookie-consent-modal > div:not([id]) > div#cookie-consent-modal-content > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div#__nuxt > div:not([id]) > div:nth-child(1):not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div#cookie_banner > div:not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div:nth-child(4):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(3):not([id]) > container:nth-child(2):not([id]) > div:not([id]) > div:not([id]) > div#versionized-cookie-banner > div:nth-child(2):not([id]) > ul:not([id]) > li:nth-child(2):not([id]) > button#btn-reject-cookies", "body:not([id]) > div#consent-banner > div:nth-child(2)#truste-consent-track > div:nth-child(3)#truste-consent-content > div:nth-child(3):not([id]) > div#truste-consent-buttons > button:nth-child(3)#truste-consent-required", 'body > div#__next > div:not([id]) > div:nth-child(1)#cookieBanner[data-testid="cookie-banner"] > div:nth-child(2):not([id]) > div:not([id]) > div:nth-child(1):not([id]) > p:not([id]) > a:nth-child(2):not([id])', "body > div[class] > div:nth-child(2) > div:nth-child(2) > button:nth-child(2)", "body > div#cmpbox > div:not([id]) > div:nth-child(1)#cmpboxcontent > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > a:not([id])", "body > div[id][cookie-banner-data-theme] > div > div > div > div:nth-child(2) > div:nth-child(2) > button", "body:not([id]) > div#seers-cmp-cookie-data-hol > div:nth-child(2)#SeersCMPBannerMainBar > div:nth-child(2):not([id]) > a:nth-child(2):not([id])", "body > div:not([id]) > div:nth-child(3)#gdpr_cookie_info_bar-wr > div:nth-child(1):not([id]) > div:nth-child(3):not([id]) > button:nth-child(2):not([id])", "body > div[id][class][data-role][data-controller][style] > div > div:nth-child(2) > form > button:nth-child(3)", "body > div[class][tabindex] > div > div > div:nth-child(2) > div:nth-child(2) > button:nth-child(2)", "body > div[id] > div:nth-child(2) > div:nth-child(4) > div > div:nth-child(2) > button:nth-child(2)", "body > div#cassie-widget > div:nth-child(4)#cassie_cookie_module > div:nth-child(2)#cassie_pre_banner > div:nth-child(3)#cassie_pre_banner__footer > button:nth-child(2)#cassie_reject_all_pre_banner", "body > div:not([id]) > div:nth-child(17):not([id]) > div:not([id]) > a:nth-child(3):not([id])", "body > aside:not([id]) > div:nth-child(1):not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", "body > div:not([id]) > div:not([id]) > div#ch2-dialog > div:nth-child(2):not([id]) > button:nth-child(1):not([id])", "body > div[class][role][aria-labelledby][aria-live][lang] > div:nth-child(1) > div:nth-child(3) > div:nth-child(2)", "body > div[id] > div > div:nth-child(1) > div > div > div > div:nth-child(2) > button:nth-child(2)", "body:not([id]) > div:not([id]) > div:not([id]) > div:not([id]) > div:nth-child(2):not([id]) > div:nth-child(2):not([id]) > button:nth-child(2):not([id])", 'button[data-qa-selector="cookie-consent-configure"]', 'label[data-qa-selector="input-box"][data-qa-selector-value="MARKETING"]', 'label[data-qa-selector="input-box"][data-qa-value="true"]:not(.input-box__disabled) input', "consent=0", ".cookiefirst-root", "#cmpwrapper", "script[src*='consent.cookiefirst.com'],.cookiefirst-root,#cmpwrapper", ".cookiefirst-root button[data-cookiefirst-action=adjust]", "button[data-cookiefirst-action=reject_second]", "#popup-cookies", "#popup-cookies .cancelar-popup", "disclaimerCookies=12=false&14=false&15=false", '#cookie-consent[style*="max-height"] #cookie-status-reject', "#b1ng-cc", "#b1ng-cc .b1ng-cc-no", ".fc-cta-do-not-consent,.fc-cta-manage-options,.fc-li-only-placeholder", "b1ng_consent=n", '[data-react-class="CookieNotice"]', '.cc-window[aria-describedby="cookieconsent:desc"]', '[data-react-class="CookieNotice"] button', '.cc-window[aria-describedby="cookieconsent:desc"] .cc-dismiss', ".cookie-notice-layout .cookie-notice-dismiss", ".ui-alert.bottom-fixed a[data-href*='cookie-policy']", ".ui-alert.bottom-fixed", ".ui-alert.bottom-fixed button.close", "body.enable-cookie-control", ".cookie-dialog-initial", ".cookie-dialog-initial button.g-button.outline", "cookie_preferences=%7B%22allow%22%3A%5B%5D", "#cookie_modal_placeholder dialog.bm-modal--cookie", "Analytics_consent=2", "Marketing_consent=2", '.MuiDialog-root:has([data-uit="accept-all"], [data-uit="store-changes"])', '[role="dialog"] #approveAllCookies[data-uit="accept-all"]', '[role="dialog"] [data-uit="show-details"]', '[role="dialog"] [data-uit="store-changes"]', '[role="dialog"] input[role="switch"]:checked', '[role="dialog"] [data-uit="accept-all"], [role="dialog"] [data-uit="store-changes"]', ".cookie-alert", ".cookie-alert .inner-cookie-alert .consent-actions", ".cookie-alert-v2", ".cookie-alert-v2 .consent-actions button:nth-of-type(2)", ".cookie-alert .consent-actions button.btn-outline-light", ".cookie-alert .consent-settings", '.cookie-alert .consent-settings input[type="checkbox"]:not([disabled]):checked', '.cookie-alert .consent-settings button[type="submit"]', "__consent=1", '*[aria-labelledby="consent-banner-title"]', "#bbccookies-prompt:has(#bbccookies-reject-button)", "#consent-banner-title", "#bbccookies-reject-button", 'button[data-testid="reject-button"]', "ckns_explicit=1", "#cookiePrompt:has(#bbccookies-continue-button)", "#bbccookies-continue-button", "#bbccookies-prompt", "#note .cpref", "#note .cpref #cPrefToggle", "#cPrefToggle", '#cPrefModal label[for="tab-4r"]', "#cPrefSave", "section.cookie", "section.cookie .cookie__container .cookie__close", "section.cookie.show .cookie__container", "section.cookie.show .cookie__close", '.cc-window[aria-describedby="cookieheading"]', ".cc-window .dp--cookie-check", ".cc-window:not(.cc-invisible) .cc-compliance", ".cc-window .cc-settings", ".cc-window .dp--check-box:checked:not(:disabled)", ".cc-window .cc-allow", '"dp--cookie-marketing":false', ".cookies-box", ".cookies-box #btn-accept-cookies", ".modalwindow-container.bar .modalwindow", "#bibliotheek-nl-page", ".modalwindow-container.bar .modalwindow button.button.primary.notok", "#b_notificationContainer.bnp_ttc", "#b_notificationContainer.bnp_ttc #bnp_ttc_close", "#bnp_ttc_close", '#thp_notf_div:has(a.hpn_top_link[href*="linkid=521839"], a.hpn_top_link[href*="bGlua2lkPTUyMTgzOQ"])', ".authorize", ".authorize .lnk-disagree", "save_open_time=ok", ".sygnal24-gdpr-modal-wrap", ".sygnal24-gdpr-modal-wrap .sygnal24-gdpr-modal", ".sygnal24-gdpr-modal-wrap .sygnal24-gdpr-options input:checked:not([disabled])", ".sygnal24-gdpr-modal-wrap .sygnal24-gdpr-accept-choice", 'gdpr-consent=["necessary"]', "#cookies.c-modal-cookies", "#cookies.c-modal-cookies .o-modal__footer", "#cookies.c-modal-cookies .o-modal__footer .c-button--secondary", "#cookies-settings.c-modal-cookies-settings .o-modal__footer", "#cookies-settings .o-list-bare__item:not(.disabled) .o-input-switch input:checked:not(:disabled)", "#cookies-settings.c-modal-cookies-settings .o-modal__footer .c-button--secondary", "cookies_consent=strictly_necessary", "[role=dialog] [data-cy=allowCookiesButton]", '[role=dialog] [data-cy=allowCookiesButton] ~ a[href="/articles/cookies"]', ".container > h3 ~ button", ".container > h3 > input[type=checkbox]:checked:not([disabled])", "BokadirektCookiePreferencesMP=Mandatory", "#cookiemodal #cookie-donottrack", "#cookiemodal:not(.cookiemodal--hidden) #cookie-donottrack", "BokioDNT=DoNotTrack", "div.cookiePolicy", "div.cookiePolicy > span[href='#']", "cookiePol=1", ".cookiebanner-container", ".cookiebanner-container [data-test=adjust-cookies]", ".cookiebanner-container .checkbox-container", ".cookiebanner-container .checkbox-container input[type=checkbox]:checked:not(:disabled)", ".cookiebanner-container .buttons-container button:not([data-test=adjust-cookies])", "cookies-accepted=false", ".keks-privacy-layer", ".keks-privacy-layer #cube-cmp-reject-all", ".keks-privacy-layer[open]", "acceptedCategories%22%3A%5B%22REQRD%22%5D", ".blp-necessary-cookies-banner", ".blp-necessary-cookies-banner__button", "has-dismissed-necessary-cookies-banner=true", "#banner-terms-dismiss", '"haveConsent":true', "#banner-terms", "dismissedTermsNotice", "#js-cookie-alert #cookie", "#js-cookie-alert.show #cookie", "#js-cookie-alert #cookie button.closeAlert", ".cookieHolder", ".cookieHolder .cookie-link", "GeneralCookieState=cookiestate", ".cookie-consent", ".cookie-consent .cookie-consent__customize", ".cookie-consent .cookie-consent__details", "#consent-analytics:checked", "#consent-analytics", ".cookie-consent .cookie-consent__save", "#cc-banner", "#cc-banner #cc-reject", '[role="dialog"][aria-modal="false"]:has(a[data-anchor-id="cookie-policy"])', '[role="dialog"][aria-modal="false"]:has(a[data-anchor-id="cookie-policy"]) button:last-child', 'div:has(> h3):has(> p a[data-anchor-id="cookie-policy"]) + div > button:last-child', 'div:has(> h3):has(> p a[data-anchor-id="cookie-policy"]) ~ div:last-child > button:only-child', "CTC=", "div.modal.cookiesModal.is-open", 'div.cookiesModal__buttonWrapper > button[data-closecause="close-by-manage-cookies"]', "button#js-manage-data-privacy-save-button", "#privacy-banner-lh .lh-close-button", "lh-warning-test=manual-no", ".cc-cookies .main", ".cc-cookies .main .btn-cookies-config button", ".cc-cookies .main[style*='display: block'] .btn-cookies-config button", "#modalConfigCookies #acceptAdditionalCookies", "#modalConfigCookies #acceptAdditionalCookies:checked", "#btnConfigCookies button", "cookie-agreed=false", "#cookie-notification", "#cookie-notification .cookie_pref_wrapper .pref-opt.refuse", "#cookie_banner #myModal", "#cookie_banner #myModal #popupCross", "#myModal #popupCross", ".incentive-banner", "#cookie-compliance", "#cookie-compliance [data-cookie-accept]", "#js-cookie-banner", "#js-cookie-banner .js-cookie-settings-btn", "#js-cookie-banner .cookie-banner-content", "#js-cookie-banner-modal .cookie-settings", "#js-cookie-banner-modal .js-cookie-setting-switch.btn-toggle-on", "#js-cookie-banner-modal .js-done-btn", "rac=1", ".cookie-banner", ".cookie-banner .all4-cc-primary-button", '.cookie-banner button[aria-label="Reject non-essential cookies and continue."]', "C4_CC=eyJ2ZXJzaW9uIjoxLCJjb25zZW50ZWQiOnRydWUs", "dialog[data-cookie-consent]:not([aria-modal]):has(form[data-privacy-consent-form])", 'dialog[data-cookie-consent] form[data-privacy-consent-form] button[name="action"][value="reject"]', "dialog[data-cookie-consent]:has(form[data-privacy-consent-form])", "dialog[data-cookie-consent]", "#usecookie", "#usecookie #usercookie_close", 'div[role="dialog"][aria-label="Cookie settings"]', 'xpath///div[@role="dialog"][@aria-label="Cookie settings"]//button[normalize-space()="Cookie settings"]', 'xpath///div[@role="dialog"][@aria-label="Cookie settings"]//button[normalize-space()="Do not accept"]', ".cookiesInfo .closeCookiesInfo", ".cookiesInfo", ".js-cookie-banner", "#js-cookie-banner__button-reject", "#js-cookie-banner__button-hide", "#csr-cookie-message", "#csr-cookie-message #accept_essential_cookies_button", "cookies_accepted=essential", ".cookie-bar-backdrop", "#js-cookie-notification .js-reject-cookie", "#js-cookie-notification.opened", "#js-cookie-notification.opened .js-reject-cookie", "cookie-notification=REJECTED", "#cookiesjsr #cookiesSize.cookiesjsr-banner", "#cookiesjsr", "#cookiesSize.cookiesjsr-banner.active", "#cookiesSize .cookiesjsr-banner--action .cookiesjsr-btn:not(.important)", "#consent-banner", "#consent-banner #truste-consent-track", "#truste-consent-required", "cmapi_cookie_privacy=permit_1_required", "#cookie-banner #customizeCookies", "#cookie-settings #saveCookies", "#analytics-cookies:checked", "#analytics-cookies", "#marketing-cookies:checked", "#marketing-cookies", "#cookie-settings", ".cookieconsent[data-init=cookieConsent]", ".cookieconsent.cookieconsent--active", ".cookieconsent [data-cookieconsent=button]", "acceptCookies=true", ".cookie-prompt-container", ".cookie-prompt-container #custom-cookie-banner-container", ".cookie-prompt-container #analytic-cookie:checked", ".cookie-prompt-container #analytic-cookie", ".cookie-prompt-container .confirm-cookies", "cookie_policy=false", "gsc-cookie-banner", "[data-consent-button][data-value=accept_partial]", 'div.fixed.z-50.rounded-xl:has(> p > a[href*="privac"]):has(> button.button-primary)', "[data-testid='cookie-banner-reject-button']", "CMCCP=AD%3D0", '#costco-consent-manager [data-testid="Popup"]', '#costco-consent-manager [data-testid="Popup"], script[src*="/consent-ui/costco-consent-manager"]', "#costco-consent-manager", '.cookie-banner-alerts > div:has(button[aria-label="Accept"])', '.cookie-banner-alerts button[aria-label="Accept"]', "#cookie-config-panel", "#cookie-banner #configure-cookies", "#cookie-config-panel #reject-cookies", "cookiesRejected=1", "#CookiesWindow", "#CookiesWindow .rodo-window", "#cookie-disclaimer", "#cookiesel", ".box-cookie", ".box-cookie .box-cookie__inner", ".box-cookie .btn-normal.is-small", "disney_cc=ok", '#cuvva-app-injection-site button[data-testid="accept-cookies"]', "cuvva.app.website.cookie-policy.consent.v2=", '#cuvva-app-injection-site button[aria-label="Reject all"]', "cuvva.app.website.cookie-policy.consent.v2={%22preferences%22:{%22functional%22:true%2C%22analytics%22:false%2C%22marketing%22:false", 'div[class*="Overlay__container"]:has(div[class*="TCF2Popup"])', 'div[class*="TCF2Popup"]', '[class*="TCF2Popup"] a[href^="https://www.dailymotion.com/legal/cookiemanagement"]', 'button[class*="TCF2ContinueWithoutAcceptingButton"]', "dm-euconsent-v2", '[class*="PrivacyBar_gdprAgreement"] [class*="PrivacyBar_managePrefsBtn"]', '[class*="PrivacyBar_gdprAgreement"]', '[class*="CookieManagementModal_confirmBtn"]', '[class*="CookieManagementModal_dropdownHeader"] [class*="SwitchButton_toggleSwitch"] input:checked', "%22targeting%22:false", "gdpr-consent-popup", '[data-rum="cookie-consent"]', '[data-rum="cookie-consent"] [data-rum="cookie-consent-text"]', '[data-rum="cookie-consent"] [data-rum="cookie-consent-text"] + div > button:first-of-type', '[data-rum="cookie-consent"] div.grid input[type="checkbox"]', '[data-rum="cookie-consent"] div.grid:has(input[type="checkbox"]:checked:not([disabled]))', '[data-rum="cookie-consent"] div.grid ~ div > button:last-of-type', "cookie_consent=necessary", 'div[role="dialog"][class*="CustomCookieBanner"]', "#onetrust-consent-sdk .ot-pc-refuse-all-handler", "ngc-cookie-banner", "div.cookie-footer-container", "#cookies-modal", "dialog.js-cookie-bar", "dialog.js-cookie-bar .js-reject-all", "CookieLevel=Functional", '.MuiDialog-root [role=dialog] button[aria-label^="Odrzu\u0107 wszystkie i zamknij popup"]', ".custom-cookiebar", ".custom-cookiebar .custom-cookiebar__link", ".cc-window.cc-visible", ".cc-window .cc-dialog", ".cc-window.cc-visible .cc-consent-require-only", "dji_consentmanager", "[id^=cookie-consent-banner]", "#cookie-consent-denied", "cookie-consent=denied", ".privacy_policy_sec", ".privacy_policy_sec .cookieinfocustom", "#cookiemodal .refuse_all_cookie", "#cookiemodal .acceptchoices", "_cookie_ads=0", "#cookiebanner.cookieAcceptBar", "#cookiebanner.cookieAcceptBar #cookiebot-container", "#cookiebot-container div.manage-preferences", "#cookiePreferencesModal-card", "#cookiePreferencesModal-card input[type=checkbox][data-type]:checked", "#cookiePreferencesModal-card #cookieModalButton", "marketing%3Afalse", 'div[class*="cookie-banner-"]', 'div[class*="cookie-banner-"] [class*="cookie-footer-"]', 'div[class*="cookie-banner-"] [class*="cookie-footer-"] > button:not([data-testid="cookie_banner_notice"])', "#ari-nw-x", "#ari-nw-x .ari-nw-btn", "#ari-nw-x button.ari-opx-btn[data-key='consent:set']", "#ss-su #ss-ck-a0", "#ss-su #ss-ck-f0", "#ss-su #ss-ck-s0", "#ss-su #ss-rl-f1-btn", "#ss-ck-f0.ari-h1", "#ss-ck-s0.ari-h1", "#ss-ck-a0.ari-h1", ".js-overlay .js-consent .js-consent-selected", ".js-consent .js-consent-option.is-selected[data-value=stat]", ".js-consent .js-consent-selected", "cks=req", "stat", '.cc-type-info[aria-describedby="cookieconsent:desc"] .cc-dismiss', "#cookiePopup [data-trigger=launchCookieDetails]", "#cookiesModal [data-trigger=saveCookiePrefs]", "cookiePrefs=", "#cookiePopup", "#cookiesModal", "#cookiesModal #functionalCookies:checked", "#cookiesModal #functionalCookies", "#cookiesModal #advertisingCookies:checked", "#cookiesModal #advertisingCookies", "#gdpr-banner", "#gdpr-banner-decline", ".cookie-wrapper", ".cookie-wrapper > .cookie-notice", "[data-test-id=cookie-notice-reject]", "#cookieBanner button[data-fragment=button]", "#cookieBanner button[data-fragment=button]:last-of-type", "[role=dialog][data-smtype=Dialog_alert] button[data-fragment=button]", "[role=dialog][data-smtype=Dialog_alert] [data-fragment=switch] input:not([disabled])[value=true]", "[role=dialog][data-smtype=Dialog_alert] button[data-fragment=button]:last-of-type", "cookies_analytics=false", "#edp-cookies-banner", "#edp-cookies-banner .edp-cookies-refuse", "edp_cookie_agree=0", "app-aviso-cookies .c-aviso-cookies", "app-aviso-cookies .c-aviso-cookies__btn", "#educacentrosPrivacyPolicyLayer", "#ef-ccpa", "#ef-button-ccpa-decline", "#huk_cookie_prefernce_panel", "#huk_cookie_prefernce_panel #EU_OPIN_REJECT", "EKD_OPTIN=1", ".cookie-bar__wrapper:has(.cookie-bar__message--top)", ".cookie-bar__wrapper .cookie-bar__message--top", ".cookie-bar__wrapper .cookie-bar__container", '[data-testid="cookie-banner"]', '[data-testid="accept-all-cookies-button"] ~ [data-testid="cookie-settings-button"]', '[data-testid="reject-all-cookies-button"]', '[data-testid="cookie-settings-button"]', '[data-testid="accept-all-cookies-button"]', ".modal:has(> .modal-content > .cookie-consent)", ".cookie-consent #cookieDisagree", "#consent-cookie-modal", "#consent-cookie-modal #cookie-selection-button", "#consent-cookie-modal input[type=checkbox]:checked:not(:disabled)", "EMR-CookieConsent-Advertising=0", "#block-eh-default-cookiebannerfixedblock", "#block-eh-default-cookiebannerfixedblock .js-cookie-alert", "#block-eh-default-cookiebannerfixedblock .js-cookie-alert .alert-close", "#gdprBanner", "#gdprBanner .js-close-btn", '#app > div:has(a[href*="business.safety.google"])', '#app > div:has(a[href*="business.safety.google"]) p + div > button', '#app > div:has(a[href*="business.safety.google"]) p + div > button:last-child', "[role=dialog] form input#MARKETING", "[role=dialog] form input[type=checkbox]:checked", "[role=dialog] form button[type=submit]", "cconsent=1000", ".cookie-banner-container", ".cookie-banner-container button.preferences", "#ccc-optional-categories .checkbox-toggle-input:checked", '"analytics":"revoked"', ".global-cookie.js-cookie-banner", ".global-cookie.js-cookie-banner .global-cookie__button", ".global-cookie.js-cookie-banner:not(.global-cookie__hidden)", "_consent_banner_read=true", "#clb.clb-container", "#clb.clb-container #moreSettings", "#clb #moreSettings", "#clb #clb-panel-2", "#clb #switch2[data-active=true]", "#clb #saveSettings", "dws01-level=1", "#es-cookie-banner", "#es-cookie-banner #es-consent-settings-btn", "#es-consent-settings-btn", "#onetrust-consent-sdk input.category-switch-handler:checked", "#rgpd", "#rgpd #rgpd_refus", "#rgpd_refus", "_ecdg=assistance_O#audience_N#targeting_N", 'xpath///div[h2[normalize-space()="Ustawienia prywatno\u015Bci i cookies"]][.//button[normalize-space()="Zgadzam si\u0119"]]', 'xpath///div[h2[normalize-space()="Ustawienia prywatno\u015Bci i cookies"]]//button[normalize-space()="Zmie\u0144 ustawienia"]', 'xpath///div[h2[normalize-space()="Ustawienia prywatno\u015Bci i cookies"]]//button[normalize-space()="Zezw\xF3l na wybrane"]', ".cookies-line", '.cookies-line a[href="/cookies-policy"]', ".cookies-line.show", "[data-cookie-hint]", "[data-cookie-hint] [data-cookie-hint-save-settings]", "[data-cookie-hint] [data-cookie-first]", "[data-cookie-hint] [data-cookie-hint-settings]", "[data-cookie-settings] #cookie-setting-1", "cookieHint=1", "cookieAllowed=1", "div#klaro", "xpath///div[contains(@class,'pointer-events-none')]//button[normalize-space()='Accept all' or normalize-space()='Tout accepter']", "xpath///button[normalize-space()='Customize' or normalize-space()='Personnaliser']", "xpath///button[normalize-space()='Decline' or normalize-space()='Refuser']", "escaparium_cookie_consent_decided=1", '[data-cy="cookies-dialog"]', '[data-cy="cookies-dialog"] [data-cy="personalize-cookies-button"]', '#personalize-menu [data-cy="accept-essentials-button"]', ".cdk-overlay-container", ".cdk-overlay-container app-esaa-cookie-component", ".btn-cookie-refuser", "#cookie-consent-banner .wt-cck--container", '.wt-cck--actions-button[href="#refuse"]', "%22cm%22%3Atrue%2C%22all1st%22%3Afalse", "#tf-modal-container:has(.cookie-modal__footer)", "#tf-modal-container .cookie-modal__footer .js-nl-modal-decline", "#tf-modal-container .cookie-modal__footer", "efrg_v1_tracking_full=0", "common-tracking-optin", "common-tracking-optin p.opt-in__text-item:nth-of-type(2) > a:last-of-type", ".privacy-bar", ".privacy-bar #close-rgpd-notice", "#close-rgpd-notice", '[role="dialog"]:has(a[href*="/policies/cookies/"]) div[aria-hidden="true"] + div[role="button"][tabindex="0"]', '[role="dialog"]:has(a[href*="/policies/cookies/"])', '[data-testid="cookie-policy-manage-dialog-decline-button"]', "#cookie-bar #cookieBarClose", '#cookie-bar[aria-hidden="false"]', "#cookie-bar #cookieBarClose .cookie-bar-close", "FSE-COOKIEBAR=hide", "a[href='/?cookie=1'] > img[title*='cookies']", "a[href='/?cookie=1']", ".c-cookie-notice.js-cookie-bar", ".c-cookie-notice.js-cookie-bar .c-cookie-notice__button", ".c-cookie-notice.js-cookie-bar.show .c-cookie-notice__button", ".fancybox-wrap:has(#cookiemanager-popup)", "#cookiemanager-popup #cookie-modal-close", 'cookieConsent={"cookie":"rejected","cookieThird":"rejected"}', "#gdrp", ".dx-cookie-banner:has(.dx-cookie-banner-close)", ".dx-cookie-banner .dx-cookie-banner-close", "CookiePolicy=OK", ".popup-container:has(.cookie-links #js-cookie-reject)", ".cookie-links #js-cookie-reject", '"marketing":false', "#cookie #cookies-marketing", "#cookie .accept-all", "#cookie a.settings", "#cookie .cookie-settings button.close", "allow_necessary=true", ".cookieCheckContainer", ".cookieCheckContainer .closeBtn", "ACP=1", "#cookie-mask", "#cookie-mask #cookieContainer", "#cookieContainer #btn-cookie-accept-edit", "#cookier-page-2 #btn-cookie-accept-nothing", 'cookie_accepted={"nothing": 1}', 'gf-notifications .notification:has(a[href^="https://policies.google.com/technologies/cookies"])', 'gf-notifications .notification a[href^="https://policies.google.com/technologies/cookies"]', 'gf-notifications .notification:has(a[href^="https://policies.google.com/technologies/cookies"]) button.notification__close', ".fk-cookie-warning", ".fk-cookie-warning .cookie-warning-content", ".fk-cookie-warning .cookie-warning-content button:nth-of-type(2)", "fk_hide_cookie_warning=true", ".cmp__consent .cmp__preferences-button", ".cmp__consent", ".cmp__service-groups__buttons button:not(.button--primary)", "C0002:0,C0003:0,C0004:0", "#nee_fpl_cookie_policy_banner", "#fpl_cookie_policy_banner_close", ".cookies-block", ".cookies-block button", "fuqer-allow-cookies=1", ".cm-custom-cookie-line", ".cm-custom-cookie-line a[onclick*='fn_show_cookie_popup']", "#cookie_popup form[name=cookie_settings]", "#cookie_popup input.consent-checkbox:checked:not([disabled])", "#cookie_popup input[type=submit][name*='save_settings']", "consent_cookie=%7B%22ad_storage%22%3A%22denied%22", "#privacy_modal", '.gfu-floating-notification-box--fixed:has([data-qa^="cookie-policy-box"])', '[data-qa^="cookie-policy-box"] [data-qa="cookie-policy-button:AcceptAll"]', '[data-qa^="cookie-policy-box"]', "#modalCookies", "#modalCookies #ck_marketing", '#modalCookies a[data-action="preferencias"]', "#modalCookies .preferencias", "#modalCookies .preferencias input[type=checkbox]:checked:not([disabled])", '#modalCookies a[data-action="aceptar"]', "gc-cookie=%7B%22estadisticas%22%3A0%2C%22marketing%22%3A0%7D", ".modal-cookie", ".cookie-consent-popup .cookie-consent-popup__decline-button", "cookie_consent_acquired=false", ".push-banner-consent", ".push-banner-consent button.close", "#editconsent", "#gu-cookie-control", "#gu_analytics_switch_off", "#gu_ads_switch_off", "#gu-consent-confirm", "gu_consent_set=true", "gu_analytics=false", "gu_remarketing=false", ".cookies-modal", ".show-cookies-modal .cookies-modal #cookies-accept", ".show-cookies-modal .cookies-modal #cookies-reject", "#banner-cookie", "#banner-cookie #banner-cookie-button", "body.with-banner-cookie #banner-cookie", "#banner-cookie-button .btn", "#cookieBanner a[href*='cookies_agreed=']", "#cookieBanner span[onclick*='cookieBanner']", ".cookie-consent-modal.active", ".cookie-consent-modal #checkbox_analiticas", ".cookie-consent-modal button.config", ".cookie-consent-modal #checkbox_analiticas:checked", ".cookie-consent-modal .btn.accept", "cookies_allowed=true", "#cookies-info", "#cookies-info > .main-container > button", "#cookies-info.active", "#privacy-banner", "#privacy-banner #privacy-banner-close", "gs_privacy_accepted=true", ".fixed [data-testid=closeCookieBanner]", "#perfModal", "#perfModal #refuseCookies", "#perfModal #refuseCookies:not(.perfModal__btn__frozen)", "OptanonAlertBoxClosed=", "[data-target='consent-wrapper']", "[data-target='consent-wrapper'] [data-action='cookies-aanpassen']", "[data-target='consent-wrapper'] .consent:not(.hidden)", "[data-target='cookies-aanpassen'] [data-action='save-cookies']", "[data-target='cookies-aanpassen'] input[type=checkbox]:checked:not(:disabled)", "cookies_consent=1", "div.fixed.inset-0.drop-shadow-2xl", 'div.fixed.inset-0.drop-shadow-2xl button[aria-haspopup="dialog"]', 'div[role="dialog"] button[type="submit"]', "cookieConsent=true", ".cookie-banner .closeme", "[data-testid=consent-banner]", "[data-testid=manage-preferences]", "[data-testid=consent-mgr-dialog] [data-ga-button=save-preferences]", '.notices--bottom_fixer .js-notice[data-notice-id="1"]', '.notices--bottom_fixer .js-notice[data-notice-id="1"] .js-noticeDismiss', 'div[role="dialog"][aria-label^="Privacy Disclosure"]', ".cookie-policy[role=dialog]", ".cookie-policy[role=dialog][aria-labelledby=cookie-policy-heading]", ".cookie-policy[role=dialog] button.clear", ".cookie-policy[role=dialog] [data-ref=marketing]", ".cookie-policy[role=dialog] .cookie-checkmark.checkable.checked", ".cookie-policy[role=dialog] button.orange", '"cookie_consent_beste_ervaring":false', "#CookieConsent", "#CookieConsentDeclined", "section.gdpr-wrapper .gdpr-button-accept", "section.gdpr-wrapper .gdpr-button-reject", "section.gdpr-wrapper", '.notification-banner:has(a[href*="/cookie-policy/"])', '.notification-banner a[href*="/cookie-policy/"]', '.notification-banner:has(a[href*="/cookie-policy/"]) button.close-notification-banner', "#consent-container #consent", "#consent button[data-bs-target='#cookieSettings']", "#cookieSettings .modal-footer", "#grantAdStorageCookies:checked", "#cookieSettings button[onclick*='saveCookieSettings(false)']", "#consent-container #consent,#cookieSettings", "#header-gdpr", "#header-gdpr #header-gdpr-in-btn button", "#header-gdpr.is-active", "gdprCookieData=ok", "#cookie.view:has(.agree .btn)", "#cookie .agree .btn", "#cookie.view", "#modal-cookies", "#modal-cookies-options", "#modal-cookies .cookies-settings-link", "#cookieMarketingSwitch:checked", "#cookieMarketingSwitch", "#modal-cookies-options a[href*='confermaImpostazioni']", "#capa_galleta", "#capa_galleta #acepto_galleta", "#notifica-cookies", "#notifica-cookies .btn-chiudi", "[data-testid=consent-banner] [data-testid=reject-button]", "COOKIE_PREF=ANL_0|PSN_0|PRTN_0", `xpath///div[contains(., "Vill du till\xE5ta anv\xE4ndningen av cookies fr\xE5n Instagram i den h\xE4r webbl\xE4saren?") or contains(., "Allow the use of cookies from Instagram on this browser?") or contains(., "Povolit v prohl\xED\u017Ee\u010Di pou\u017Eit\xED soubor\u016F cookie z Instagramu?") or contains(., "Dopustiti upotrebu kola\u010Di\u0107a s Instagrama na ovom pregledniku?") or contains(., "\u0420\u0430\u0437\u0440\u0435\u0448\u0438\u0442\u044C \u0438\u0441\u043F\u043E\u043B\u044C\u0437\u043E\u0432\u0430\u043D\u0438\u0435 \u0444\u0430\u0439\u043B\u043E\u0432 cookie \u043E\u0442 Instagram \u0432 \u044D\u0442\u043E\u043C \u0431\u0440\u0430\u0443\u0437\u0435\u0440\u0435?") or contains(., "Vuoi consentire l'uso dei cookie di Instagram su questo browser?") or contains(., "Povoli\u0165 pou\u017E\xEDvanie cookies zo slu\u017Eby Instagram v tomto prehliada\u010Di?") or contains(., "Die Verwendung von Cookies durch Instagram in diesem Browser erlauben?") or contains(., "Sallitaanko Instagramin ev\xE4steiden k\xE4ytt\xF6 t\xE4ll\xE4 selaimella?") or contains(., "Enged\xE9lyezed az Instagram cookie-jainak haszn\xE1lat\xE1t ebben a b\xF6ng\xE9sz\u0151ben?") or contains(., "Het gebruik van cookies van Instagram toestaan in deze browser?") or contains(., "Bu taray\u0131c\u0131da Instagram'dan \xE7erez kullan\u0131m\u0131na izin verilsin mi?") or contains(., "Permitir o uso de cookies do Instagram neste navegador?") or contains(., "Permi\u0163i folosirea modulelor cookie de la Instagram \xEEn acest browser?") or contains(., "Autoriser l\u2019utilisation des cookies d\u2019Instagram sur ce navigateur ?") or contains(., "\xBFPermitir el uso de cookies de Instagram en este navegador?") or contains(., "Zezwoli\u0107 na u\u017Cycie plik\xF3w cookie z Instagramu w tej przegl\u0105darce?") or contains(., "\u039D\u03B1 \u03B5\u03C0\u03B9\u03C4\u03C1\u03AD\u03C0\u03B5\u03C4\u03B1\u03B9 \u03B7 \u03C7\u03C1\u03AE\u03C3\u03B7 cookies \u03B1\u03C0\u03CC \u03C4o Instagram \u03C3\u03B5 \u03B1\u03C5\u03C4\u03CC \u03C4\u03BF \u03C0\u03C1\u03CC\u03B3\u03C1\u03B1\u03BC\u03BC\u03B1 \u03C0\u03B5\u03C1\u03B9\u03AE\u03B3\u03B7\u03C3\u03B7\u03C2;") or contains(., "\u0420\u0430\u0437\u0440\u0435\u0448\u0430\u0432\u0430\u0442\u0435 \u043B\u0438 \u0438\u0437\u043F\u043E\u043B\u0437\u0432\u0430\u043D\u0435\u0442\u043E \u043D\u0430 \u0431\u0438\u0441\u043A\u0432\u0438\u0442\u043A\u0438 \u043E\u0442 Instagram \u043D\u0430 \u0442\u043E\u0437\u0438 \u0431\u0440\u0430\u0443\u0437\u044A\u0440?") or contains(., "Vil du tillade brugen af cookies fra Instagram i denne browser?") or contains(., "Vil du tillate bruk av informasjonskapsler fra Instagram i denne nettleseren?")]`, "xpath///button[contains(., '\u041E\u0442\u043A\u043B\u043E\u043D\u0438\u0442\u044C \u043D\u0435\u043E\u0431\u044F\u0437\u0430\u0442\u0435\u043B\u044C\u043D\u044B\u0435 \u0444\u0430\u0439\u043B\u044B cookie') or contains(., 'Decline optional cookies') or contains(., 'Refuser les cookies optionnels') or contains(., 'Hylk\xE4\xE4 valinnaiset ev\xE4steet') or contains(., 'Afvis valgfrie cookies') or contains(., 'Odmietnu\u0165 nepovinn\xE9 cookies') or contains(., '\u0391\u03C0\u03CC\u03C1\u03C1\u03B9\u03C8\u03B7 \u03C0\u03C1\u03BF\u03B1\u03B9\u03C1\u03B5\u03C4\u03B9\u03BA\u03CE\u03BD cookies') or contains(., 'Neka valfria cookies') or contains(., 'Optionale Cookies ablehnen') or contains(., 'Rifiuta cookie facoltativi') or contains(., 'Odbij neobavezne kola\u010Di\u0107e') or contains(., 'Avvis valgfrie informasjonskapsler') or contains(., '\u0130ste\u011Fe ba\u011Fl\u0131 \xE7erezleri reddet') or contains(., 'Recusar cookies opcionais') or contains(., 'Optionele cookies afwijzen') or contains(., 'Rechazar cookies opcionales') or contains(., 'Odrzu\u0107 opcjonalne pliki cookie') or contains(., '\u041E\u0442\u0445\u0432\u044A\u0440\u043B\u044F\u043D\u0435 \u043D\u0430 \u0431\u0438\u0441\u043A\u0432\u0438\u0442\u043A\u0438\u0442\u0435 \u043F\u043E \u0438\u0437\u0431\u043E\u0440') or contains(., 'Odm\xEDtnout voliteln\xE9 soubory cookie') or contains(., 'Refuz\u0103 modulele cookie op\u0163ionale') or contains(., 'A nem k\xF6telez\u0151 cookie-k elutas\xEDt\xE1sa')]", ".js-cookie-message", ".js-cookie-message #cookie-chiudi", "ISP_Cookie=1.1", "IntesaSanpaoloTracciatura=1", "#cookiebox", "#cookiebox a.close", "#layout div:has(> div > div > span > button[title='EXT_COOKIE_POLICY'])", "#layout div:has(> span > button[title='EXT_COOKIE_POLICY']) + div button", "pkobp-cookie-policy=true", '.overlayContent a[onclick^="optin"]', ".overlayBG", ".header-slideout-overlay-cookie", ".pop-cookie", "#rti-privacy-open-modal-btn-id", "#rti-privacy-close-btn-id", "#iworks_cookie_notice", "iWorks_Footer_Notice_close=hide", "body.loaded", "#iworks-footer-notice-button-close", "#cp-wrapper", "#cp-wrapper #cp-small", "#cp-small button[onclick*='cp-large']", "#cp-large", "#cp-large input[type=checkbox]:checked:not([disabled])", "#cp-large form > input[type=button]:not([onclick*='setConsentAll'])", "[data-testid=modal_cookie-consent]", "[data-testid=modal_cookie-consent] .modal-bottom button.jt-btn-outline", "[data-testid=modal_cookie-consent] #marketingConsent", "[data-testid=modal_cookie-consent] input[type=checkbox]:checked:not([disabled])", "[data-testid=modal_cookie-consent-accept]", "jt-cookie-consent-extended=%7B%22ad_storage%22%3Afalse%2C%22analytics_storage%22%3Afalse", "div.fixed:has(#marketingCookies)", "#marketingCookies", '#statisticCookies[aria-checked="true"], #marketingCookies[aria-checked="true"]', "xpath///div[.//button[@id='marketingCookies']]//button[normalize-space()='Auswahl best\xE4tigen']", "cookieChoiceDone=true", "acceptMarketing=false", "acceptStatistic=false", "#cookiebanner-popup", "#jg-chrome-header", "#cookiebanner-popup #accept-essential-Cookies", "necessary:true%2Cpreferences:false", ".consent-banner", ".consent-banner .consent-banner__actions button.basic-button", ".consent-banner__actions button.basic-button.secondary", ".consent-modal__footer button.basic-button.secondary", ".consent-modal .base-modal__body a", "label.consent-switch input[type=checkbox]:checked", ".consent-modal__footer button.basic-button.primary", ".s_box_avvisocookie", ".s_box_avvisocookie a[onclick*=juzaphoto_refuse_all_cookies]", "juzaphoto_refuse_all_cookies=ok", "xpath///div[@data-nosnippet][contains(., 'uses cookies from Google')]", "xpath///div[@data-nosnippet][contains(., 'uses cookies from Google')]/following-sibling::div/div[contains(., 'Got it')]", "ACCEPTED_COOKIES=true", ".content-container.gdpr-banner, .content-container.non-gdpr-banner", ".gdpr-banner .customize-button", ".non-gdpr-banner .privacy-text-content button", "[role=dialog] .confirm-choices", "[role=dialog] input.toggle-switch[aria-labelledby=performance]:checked", "[role=dialog] input.toggle-switch[aria-labelledby=research]:checked", "[role=dialog] input.toggle-switch[aria-labelledby=remarketing]:checked", '#root > .chakra-stack:has(> div > p > a[href="/cookies"])', 'button[aria-label^="Tilpass informasjonskap"]', 'xpath///button[contains(., "Lagre val")]', '.kvib-switch__root[data-state="checked"]', "#react-consent-modal", "#react-consent-modal .flex.flex-column", "#react-consent-modal .flex.flex-row.justify-between button:first-child", ".cookie:has(.cookie__inner .cookie__button)", ".cookie > .cookie__inner > .cookie__button > a", "#loader.is--show", ".cookie", "cookiesubmit=1", "#ccpa-request-consent", "#ccpa-request-consent .settings button", "#ccpa-request-consent.ccpa-banner--visible .settings button", ".ReactModal__Content:has(.toggle-grid) .modal__footer button.button--vibrant--filled", ".ReactModal__Content:has(.toggle-grid)", "#ccpa-request-consent button.close", "#gdpr-banner-container", "#gdpr-banner-container #gdpr-banner [data-testid=gdpr-banner-decline-button]", "#cookie-notice #close-notice", "notice=true", "#cookie-notice-full-consent", "#cookie-notice-full-consent .cn-accept-all-cookies", ".cookieconsent .cc_banner-wrapper", ".cookieconsent .cc_banner-wrapper [onclick*='refuseAllCookiesPreferences']", ".cookieconsent .cc_banner", ".cc_banner [onclick*='refuseAllCookiesPreferences']", "_cprefs=%7B%22stats%22%3A1%2C%22fct%22%3A1%2C%22mkt%22%3A1%7D", "#qc-cmp2-ui, #qc-cmp2-usp", '#qc-cmp2-usp .qc-usp-ui-content button[mode="primary"]', "#qc-cmp2-mspa-success-modal-button-footer-container button", "#qc-cmp2-container button.qc-cmp2-link-inline", "#reject-all-btn", "#save-and-exit", "#js-cookie-section", "#js-cookie-section #js-cookie-consent-deny", "#js-cookie-section.show", "cookieConsent=false", "[data-controller=cookie-bar]", "[data-controller=cookie-bar].show", "[data-controller=cookie-bar] [data-action='click->cookie-bar#customize']", "[data-controller=cookie-bar] [data-cookie-bar-target=consentOptions]", "[data-cookie-bar-target=analytics]:checked,[data-cookie-bar-target=preferences]:checked,[data-cookie-bar-target=marketing]:checked", "[data-controller=cookie-bar] [data-action='click->cookie-bar#acceptSelection']", "#gdpr", "#gdpr #gdpr-button", "#showcookie", "#showcookie #acceptCookie", "div:has(> div > a[href*='/privacy-policy'])", '[data-testid="CookieModal"], .klaro > .cookie-modal', '[data-testid="CookieModal-ConsentRefuse-Button"]', "LeetchiRGPDCookie=%7B%22security%22%3Atrue%2C%22analytics%22%3Afalse%2C%22marketing%22%3Afalse", ".cookie-preferences-banner", ".cookie-preferences-banner .reject-all-cookies", ".cookie-preferences-banner .dismiss-banner", "analytics%22%3Afalse", '[class*="ConsentModal_consent-modal__modal-overlay"]', '[data-test="cookie-necessary-button"]', '[data-test="age-gate-overlay"]', "AGE_GATE=", "%22key%22%3A%22analytics%22%2C%22value%22%3Afalse", '[class*="CookiesBanner_cookies-banner"]', '[class*="CookiesBanner_cookies-banner"] [class*="CookiesBanner_buttons"] button', '[class*="CookiesBanner_buttons"] button:first-child', '[role="dialog"] [class*="_actions_"] button:first-child', "#cookieNotice > a.btncookieNoticesettings", "section.consent-banner", "section.consent-banner button.hide-consent", "#cookiebar.mod_cookiebar", ".cookieBar-filler", "#cookiebar[data-component='bw.bnl.tracking']", "#cookiebar.noticeActive .modalwindow-container.bar", "#cookiebar button.notok", "trackingapprovedv3=false", "#cookie-consent-popup", "#cookie-consent-popup #cookie-consent-accept-selected", "#cookie-consent-popup input[type=checkbox]:checked:not(:disabled)", "analytics_cookies=false!personalizationAndMarketing_cookies=false", ".consent-categories-modal-wrapper", "#consent-categories-modal [data-modal-consent-open-settings]", "#edit-consent-app input[name=consent-category][value=nc]:checked", "#edit-consent-app input[name=consent-category]:checked:not(:disabled)", "#consent-categories-settings [data-modal-submit]", "#consent-categories-modal", "#cookie-notice:has(.cookie-btn[onclick])", "#cookie-notice .cookie-btn[onclick]", "#cookie-notice[style*='visibility: visible'] .cookie-btn[onclick]", "accepts=Y", "#cookie-banner #cookie-consent-settings", "#cookie-banner #cookie-consent-view", "#cookie-banner #cookie-settings-view", "#cookie-form input[type=checkbox]:checked:not(:disabled)", "#cookie-banner #cookie-settings-save", "analytic%3A0", "#cookieChoice", "#cookieChoice a[href='#modalCookieChoice']", "#modalCookieChoice", "#modalCookieChoice input[type=checkbox]:checked", "#modalCookieChoice button[onclick*=cookie_statistici]", "loeschertechcookieadv=1", ".cookie-overlay", "#cookiePopup .reject-cookie", "_site_acceptance=0", "div:has(> #notice-cookie-block)", '[data-dialog-ref-name="cookie-text-popup"]', "#notice-cookie-block #btn-manage-cookie", "#btn-manage-cookie-save-preferences", '#cookie-text-popup input[type="checkbox"]:checked', "%22marketing%22%3Afalse%2C%22analytic%22%3Afalse", "app-cookie-modal .top-info-cookie", "app-cookie-modal .top-info-cookie .close-x", "PatientPortalCookieMonit=1", ".js-gdpr-banner-top-hide", ".js-gdpr-banner .js-gdpr-banner-top-hide", ".js-gdpr-banner-top-hide .js-gdpr-banner-disagree", "cookie_banner=false", '.popuppanel4 a[href*="/info/cookie/"]', ".popuppanel4 .js-popupclose", ".popuppanel4", ".c-cookie-modal", ".c-cookie-modal #cookieModal", '.c-cookie-modal__buttons--container button[aria-label="Settings"]', ".c-cookie-modal__options", '.c-cookie-modal__options input[role="switch"]:checked:not([disabled])', '.c-cookie-modal__buttons--container button[aria-label="Save Cookie Settings"]', ".makita-footer-warning", ".makita-footer-warning .js-makita-modal[data-src*='privacysettings']", ".about-cookies a[data-post*='allowcookie=false']", ".cookies-notification-wrapper .cookie-accept-all-cta", ".cookies-notification-wrapper .customise-cookies-cta button", ".cookies-notification-wrapper .cookie-details-section", ".cookies-notification-wrapper .cookie-details-section input[type=checkbox]:checked:not(:disabled)", ".cookies-notification-wrapper .cookies-notification-bottom div:not(.hideCookiesPreferenceBtn) > button", "#cookies-banner", '#cookies-banner [class*="CookiesFooter-module"]', '#cookies-banner [class*="CookiesFooter-module"] > button:nth-child(2):not([id])', '#cookies-banner [class*="CookiesFooter-module"] > button:first-child:not([id])', '#cookies-banner [class*="CookiesSettings-module"][class*="buttonsWrapper"] > button:nth-child(2)', '"targeting":"DENIED"', "#Modal:has(.DlgCGU)", ".DlgCGU .DlgCGUBtn", ".DlgCGU .DlgCGUBtn button.Red", "UserAgreement=0", ".navbar-privacy", "nav.navbar-privacy .privacy-continue", "#generic-modal .privacy-accept", "nav.navbar-privacy", '.cookie-banner[data-block-name="cookie-banner"]', '.cookie-banner[data-block-name="cookie-banner"] .alert-close', ".cookie_info", ".cookie_info > span.cookie_ok", 'section[aria-label="Cookie preferences"]', 'xpath///section[@aria-label="Cookie preferences"]//button[normalize-space()="Cookie Settings"]', 'div[role="dialog"][aria-labelledby="consent-settings-title"]', 'xpath///label[contains(., "Necessary and functional cookies")]//input[@name="cookie-tier"]', 'div[role="dialog"][aria-labelledby="consent-settings-title"] button:not([aria-label])', ".privacy-license-bar", ".privacy-license-bar .privacy-btn-wrapper", 'div:has(> div > div > div[role=alert] > a[href^="https://policy.medium.com/medium-privacy-policy-"])', ".GDPRPanel-module-root", ".GDPRPanel-module-root .GDPRPanel-module-dismiss", "dialog.cookie-banner", "dialog.cookie-banner .customise-cookie", "dialog.cookie-banner button.reject", "dialog.cookie-dialog button.save", "dialog.cookie-dialog .toggle-control.on:not(.disabled)", "div.z-notification:has(a[href*='mentimeter.com/cookie-policy'])", "div.z-notification a[href*='mentimeter.com/cookie-policy']", "div.z-notification:has(a[href*='mentimeter.com/cookie-policy']) > button", "has_approved_cookie_policy", "div[role=region][aria-label='Cookie consent']", "div[role=region][aria-label='Cookie consent'] button", "nia_consent_seen=1", "dialog.consent-dialog .consent-settings .consent-category input[data-category]", "dialog.consent-dialog", "dialog.consent-dialog .consent-wrapper > .consent-buttons > .consent-button-secondary", "dialog.consent-dialog .consent-settings.active", "dialog.consent-dialog .consent-category input[type=checkbox]:checked:not([disabled])", "dialog.consent-dialog .consent-settings .consent-buttons > .consent-button:not(.consent-button-secondary)", "%22ad_storage%22%3A%22denied%22", "xpath///div[@role='dialog' and @aria-modal='true']//div[following-sibling::div[@aria-hidden='true'][.//div[@role='button' and @aria-disabled='true']]]/div[@role='button' and @tabindex='0']", 'div[aria-label="Cookie Policy Banner"]', "dialog.cookieconsent__dialog", '.cookieconsent__base[data-component="cookieconsent"]', ".cookieconsent__buttonAcceptNecessary", "consent_choice=", ".cookie-modal.open", ".cookie-modal.open .cookie-box", ".cookie-modal.open .btn-box .simple-btn.cancel", "#avisoUsoCookies", "#avisoUsoCookies .configurar-cookies", "#cookies-modal .cookies-btn-guardar", "#cookies-modal input[id^=check-]:checked:not(:disabled)", "TRABAJO_POLICYv2=true", "TRABAJO_GA=", "#consent-tracking .btn-necessary", '"marketingCookie":false', "#mde-consent-modal-container", "#mde-consent-modal-dialog", "#mde-consent-modal-dialog .mde-consent-decline-btn", "mdeConsentData=", "[role='region'][aria-label='Cookie consent']", "xpath///div[@role='region'][@aria-label='Cookie consent']//button[normalize-space(.)='Reject']", "[role='dialog'][data-diagnostic-boundary='modal-shell'] button[aria-label='Close']", "#cookie-confirm", "#cookie-confirm a[href*='cookieConfirmClose2']", "#cookie-banner-wrapper", "#cookie-banner-wrapper #cookie_banner_save", "#cookie-banner-wrapper input[type=checkbox]:checked:not(:disabled)", "analytical_cookie_consent=0", "third_party_cookie_consent=0", ".cookies:has(> #agreeButton)", ".cookies > #agreeButton", "mq-alert-cookies", ".v-cookie-bar", ".v-cookie-bar__initial-state .c-button.--secondary", ".v-cookie-bar__settings", ".v-cookie-bar__settings input[type=checkbox]:checked:not(:disabled)", ".v-cookie-bar__settings .u-text-center .c-button", "#cmp-banner-sdk", ".cmp-sdk-container", "#cmp-reject-all-handler", ".cookie-new-bg", "#cookie-popup-short", "#cookie-popup-short .manage-cookies", "#cookie-popup-detailed .save-settings", "#cookie-popup-detailed input.cookie-toggle:checked", "#onetrust-banner-sdk", ".privacy-confirmation", ".privacy-confirmation .pc-buttons", ".privacy-confirmation [data-cy=accept-core-button], .privacy-confirmation .pc-buttons > button.ui-parts-button-fill:not([data-cy])", 'dialog[data-test="common-consent"]', "#nlportal-cookie-consent .modal", 'dialog[data-test="common-consent"] [data-test="consent-reject-all"], #nlportal-cookie-consent .cookie-consent-modal-static-buttons > button', 'dialog[data-test="common-consent"] [data-test="consent-reject-all"], #nlportal-cookie-consent .cookie-consent-modal-static-buttons > button:nth-child(2)', 'dialog[data-test="common-consent"], #nlportal-cookie-consent .cookie-consent-modal-static-buttons', ".cookie-policy-wrapper .cookie-policy-content", ".cookie-policy-wrapper #declineCookies", "cookies_policy=analyticsCookies%3Doff%26functionalityCookies%3Doff", "div#cookieWarning", "a#btnCookiesDenyAll", "button[aria-label='Close cookie banner']", "cookie-banner", '[data-show-cookie-banner="true"]', '[data-show-cookie-banner-overlay="true"]', '[data-show-cookie-banner="true"], [data-show-cookie-banner-overlay="true"]', "[class*=ConsentManager]", "[class*=ConsentManager_cookieBar]", 'xpath///button[contains(@class, "ConsentManager_cookieBarButton") and contains(., "weigeren")]', "#cookie-banner #reject-cookies", "#cookie-banner.nhsuk-cookie-banner", '#cookie-banner [data-qa="cookie-banner-close"]', '#cookie-banner [data-qa="cookie-banner"]', ".cookies:has(.closecookies)", ".cookies .closecookies", "[data-testid=cookie-dialog-root]", "input[type=radio][id$=-declineLabel]", "[data-testid=confirm-choice-button]", "div.fixed.opacity-100 button[aria-label='Toggle Essential cookies']", "div.fixed.opacity-100:has(button[aria-label='Toggle Essential cookies']) button[aria-label='Close']", "#cookiebar #acceptallpolicies", "form input[name=cookiepolicy][value=THIRDPARTY]", "form input[name=cookiepolicy]:checked:not(:disabled)", "form:has(input[name=cookiepolicy]) button[type=submit]", "#cookiebar a[href*='Privacy_en_cookies/Cookies']", "#CookieReportsPanel #CookieReportsBanner .wscrOk2", "#CookieReportsBanner .wscrSwitch input[type=checkbox]:checked:not([disabled]) + label", "#CookieReportsBanner .wscrOk2", "wscrCookieConsentShared=1=true&2=false&3=false&4=false&5=false", "[role=dialog] #cookie-accept-all-secondary", "[role=dialog] #cookie-accept-all-secondary + button", "consent_cookie=necessary", "nwhlit-gdpr-banner", "#consent.container-banner-cookie .consentOk", "consent=", ".cookie-control", '.cookie-control [data-cc-capture] button[name="save"]', ".cookie-control .cc-capture__message", ".cookie-control [data-expandable-trigger]", '.cookie-control input[name="cookie_control_optin_level[]"]:checked', "cookie_control_optin_level=0", "gdpr-banner", "[class^='cookies_cookies__']", "[class^='cookies_cookies__'] [class^='cookies_close__']", "xpath///button[contains(., 'Imposta le tue preferenze')]", "input[data-part='checkbox-input'][name='analytics']", "input[data-part='checkbox-input'][name='analytics']:checked", "input[data-part='checkbox-input'][name='marketing']:checked", "input[data-part='checkbox-input'][name='marketing']", "xpath///button[contains(., 'Salva le mie impostazioni')]", "cookie_consent=a%3D0%26m%3D0", ".cookie-banner:has(.cookie-banner__close)", ".cookie-banner .cookie-banner__close", ".cookies-agreement-notification,.modal-new:has([data-module=SetupCookies])", ".cookies-agreement-notification", ".cookies-agreement-notification .cb_setup", "[data-module=SetupCookies]", "[data-module=SetupCookies] input[type=checkbox]:checked:not(:disabled)", "[name=button_save_choice]", "#cookiePolicy > menu", "aside:has(#featureSwitch_necessary)", "#featureSwitch_necessary", "xpath///aside[.//*[@id='featureSwitch_necessary']]//button[contains(., 'Dostosuj')]", "#featureSwitch_marketing", "#featureSwitch_marketing[data-state='checked']", "xpath///aside[.//*[@id='featureSwitch_necessary']]//button[contains(., 'Potwierd\u017A wyb\xF3r')]", "privacy2=necessary", "#gdpr-selection-screen", "#gdpr-selection-screen #gdpr-permissionlevel-1", "#gdpr-selection-screen #gdpr-screen-content", "#gdpr-selection-screen .button-container button", "MascGdrpPermission=1", "section.cookie-note[role=region] > button", "section.cookie-note[role=region]", ".fn-cookiewall-container, .fn-cookiewall-banner-container", ".fn-cookiewall-container", ".fn-cookiewall-container .cookiewall--reject-btn", ".fn-cookiewall-container .cookiewall--pre-submit-btn", ".fn-cookiewall-container .cookiewall--submit-btn", ".fn-cookiewall-banner-container", ".fn-cookiewall-banner-container .cookiewall-banner-close", "#alert_cookies", "#alert_cookies #alert_cookies_close", "div.b-cookies-informer", "div.b-cookies-informer__nav > button:nth-child(1)", "div.b-cookies-informer__switchers", "div.b-cookies-informer__switchers input:not([disabled])", "div.b-cookies-informer__nav > button", ".cookie-popup", ".cookie-popup .cookie-popup__button", "[aria-labelledby=cookieConsentTitle]", "[aria-labelledby=cookieConsentTitle] button[aria-haspopup=dialog] + button", "consent=rejected", "#cookie-consent .cookie-all__btn", "#cookie-consent .cookie-consent__switch.active:not(.always_on)", "#cookie-consent .cookie-selection__btn", "cookie_consent_essential=true", "cookie_consent_marketing=true", '.o5-dynamic-alerts__container:has(a[href*="privacy-policy"])', '.o5-dynamic-alerts__container:has(a[href*="privacy-policy"]) .o5-dynamic-alerts__close', "#id01.w3-modal", `#id01.w3-modal button[onclick*="setCookie('polityka'"]`, "#orbys-cookies", "#orbys-cookies .occ-btn", "#consent-manager-modal", "#consent-manager-modal #cookies-consent", "#consent-manager-modal [role=dialog]", "#cookies-consent label[role=checkbox]:has(input:checked)", "#consent-manager-modal [class*=modal_buttons] button[type=submit]", "osf-cookie-consent-banner", "osf-cookie-consent-banner p-message", "#footerCurtainCookie", "#footerCurtainCookie .footerCookieLink", "SL_cookie=NO", ".cookie-manager", ".cookie-manager .cookie-notice.open", ".cookie-notice button.owid-btn--outline-dark-blue.cookie-notice__button", "#ccpm", "html.show--consent #ccpm .cm__btn[data-role=all]", "#ccpm .cm__btn[data-role=show]", "#ccpm .pm__btn[data-role=save]", "#ccpm .pm input.section__toggle:checked:not(:disabled)", "#cookie-consent ~ .banner-shadow", '#cookie-consent .cc-window[aria-describedby="cookieconsent:desc"]', "#cookie-consent, #cookie-consent ~ .banner-shadow", "footer .ccpabanner", "#bandeaucookie", "#bandeaucookie #cookieOk", ".cookie-consent-wrapper", ".cookie-consent-wrapper a[color=transparent]", "input[name=ga-cookies]", "input[name=ga-cookies]:checked, input[name=fbq-cookies]:checked", "[class*=CookieConsentModal__ButtonContainer] a:not(.cancel-btn)", "cookieconsent=0", ".cc-policy-overlay", ".cc-policy", ".cc-policy .cc-settings", "consent=S.false-T.false", "pekao-cookies", "pekao-cookies .cookies-wrapper", "pekao-cookies .cookies-button button", "np24Cookies=", "[data-ui-test=pm-cookie-banner]", "[data-ui-test=pm-cookie-banner-decline-button]", "pm_cookie_consent_level=1", "#cookies-info .btn-cookie-close", "cookieInfoSZG=true", ".cookie-consent-popup .cookie-consent-accept", ".cookie-consent-popup .cookie-consent-accept .elementor-button", "cookie_consent=accepted", ".BusinessCookieConsent", ".BusinessCookieConsent [data-id=cookie-consent-banner-buttons]", "[data-id=cookie-consent-banner-buttons] > div:nth-child(2) button", "#cookieMessage[data-cookie-boundary]", ".cookie-bg", ".cookie-bg .cookie-inner .cookie-btn--accept", ".cookie-bg .cookie-inner", ".cookie-bg .personalise-btn", ".cookie-bg form .cookie-list", ".cookie-bg form .cookie-item input[type=checkbox]:checked:not([disabled])", ".cookie-bg form button[type=submit]", "gdpr-ga=false", "gdpr-fbp=false", "[data-testid=cookieBar]", ".notification-bar.cookie-consent", ".notification-bar-mobile.cookie-consent", "[data-testid=adConsentModal]", "[data-testid=adConsentDisagreeButton]", "[data-testid=cookieBarRejectButton]", ".notification-bar-mobile.cookie-consent.remodal-is-opened .cookie-consent-reject", "[data-testid=cookieBarRejectButton], .notification-bar.cookie-consent .cookie-consent-reject", "plex_tv_cookie_consent=2", "#cookie-consent button", "#cookie-consent input[type=checkbox]:checked:not(:disabled)", "plosCookieConsentStatus=false", "[data-testid=cookie-consent-bar-desktop], [data-testid=cookie-consent-bar-mobile]", "[data-testid=manage-cookies-button-desktop]", "[data-testid=manage-cookies-button-mobile]", "[data-testid=cookie-consent-modal]", "[data-testid=cookie-consent-modal] input[type=checkbox]:checked", "[data-testid=cookie-consent-save-button]", "pmo_c_acc=true", "pmo_c_mark=true", "#notification-stack > div:has(> header > [style*='ui-icons-lock.svg'] ~ button)", "#notification-stack header:has([style*='ui-icons-lock.svg']) button", "#notification-stack header:has([style*='ui-icons-lock.svg'])", "#cookieBanner.cbShort", "#cookieBanner.cbShort .cbCloseButton", "#cookieBanner #cookieBannerContent", "#cookieBanner #cookieBannerContent, #globalCookieBanner", "#globalCookieBanner .js-customizeGlobalCookies", "#cookieBanner [data-label=accept_essential]", "#cookieBanner button.cbSecondaryCTA", "#cookie-form", "#cookie-form .reject", "_cookieNoticeSettings=%7B%22performance%22%3Afalse%2C%22targeting%22%3Afalse%2C%22analytic%22%3Afalse%7D", "xpath///li[contains(@class, 'ToastRoot')][contains(., 'cookie banner')]", "xpath///li[contains(@class, 'ToastRoot')][contains(., 'cookie banner')]//button[@data-radix-toast-announce-alt='Close']", "pnl-cookie-wall-widget", "CookiePermissionInfo", ".alert .accept-cookies,form.js-cookies", ".alert:has(.accept-cookies)", "form.js-cookies button", "form.js-cookies input[type=checkbox]", "form.js-cookies input[type=checkbox]:checked:not(:disabled)", "form.js-cookies button[type=submit]", ".alert:has(.accept-cookies) a[href='/account/cookies']", "#cookiesPopup", "#cookiesPopup button.MuiButton-outlined", "#cookiesPopup .MuiSwitch-root", "#cookiesPopup button", "#tc-banner", "#tc-consent #tc-banner", "#tc-banner .tc-accept-btn", ".tc-checkbox:checked:not([disabled])", 'UserConsentedToTastyCookies=["nodvandiga"]', ".c-modal.is-active", ".c-modal.is-active .is-dismiss", '.cc-window.cc-type-info[aria-describedby="cookieconsent:desc"] .cookieconsent__dismiss', '.cc-window[aria-describedby="cookieconsent:desc"] .cookieconsent__dismiss', "div.cookie:has(a.cookie-okay-button)", "div.cookie a.cookie-okay-button", ".consent:has(> .consent-body .consent-buttons)", ".consent > .consent-body .consent-buttons > .btn-outline-secondary", ".consent > .consent-body .consent-buttons", ".ReactModalPortal", ".consent-settings-modal .consent-switchers", ".consent-settings-modal .consent-switchers input[type=checkbox]:checked:not([disabled]) + label", ".consent-settings-modal .consent-settings-modal-content > .consent-buttons > .btn-primary", '[data-testid="cookie-consent-sticky-footer"]', '[data-testid="cookie-consent-sticky-footer"][aria-hidden="false"]', '[data-testid="cookie-consent-sticky-footer"] button[aria-label]:first-child', '[data-testid="_cookie_modal_test"]', '[data-testid="_cookie_modal_test"] label:has(input[type="checkbox"]:checked)', '[data-testid="_cookie_modal_test"] button[aria-label="button-pmui"]', '"analytics_storage":"denied"', ".cookies .cookies__container", ".cookies .cookies__close button.cookies__button", "cookieAgreements=true", "#popup.cookie-popup", "#popup.cookie-popup #close-popup", ".gdpr[data-gdprservice]", ".gdpr[data-gdprservice]:not(.hide)", "#cookie-banner #cookie-banner-statistics", "#cookie-banner-preferences:checked", "#cookie-banner-preferences", "#cookie-banner-statistics:checked", "#cookie-banner-statistics", `#cookie-banner button[onclick*="'selected'"]`, "RDOPRIVACY=%5Btrue%2Cfalse%2Cfalse%5D", "#__rptl-cookiebanner #__rptl-cookiebanner-reject", ".DialogHandlerContainer.visible:has(.cookie-dialog)", "#cookiesStrip:has([data-testid='accept-cookies'])", ".DialogHandlerContainer.visible:has(.cookie-dialog), #cookiesStrip:has([data-testid='accept-cookies'])", "#cookiesStrip [data-testid='skip-cookies']", "span.cookie-option-large>div>span.link-no-decoration.gui-text", ".cookie-dialog form .CookieConsentOption input[type='checkbox']:not([name='permanent']):checked", ".cookie-dialog button[type='submit']", "[bundlename=reddit_cookie_banner]", "#data-protection-consent-dialog rpl-modal-card > button.button-primary", "#data-protection-consent-dialog rpl-modal-card > button.button-secondary", "eu_cookie={%22opted%22:true%2C%22nonessential%22:false}", "#cookiebanner", "#cookiebanner #acceptNecessaryCookiesBtn", "refbConsent={%22categories%22:[%22necessary%22]%2C", "%22consent_date%22:1", "#cb_wrapper-std #overlay-std", "#cb_wrapper-std #cookieApiData", "#cb_wrapper-std #overlay-std #userSelect", "#cb_wrapper-std #userSelect", "#footer > .container > .d-flex > button.btn-outline-primary", "div:has(> button#cookieBar-button)", "#cookieBar-button", "#cookie-consent-container[style*='visibility'] #cookie-banner", "#cookie-banner .cookie-banner-preferences-btn", "#cookie-consent-detail", "#cookie-consent-detail .modal-content-agree-to-preference-btn", "#clawackdiv", "#clawackdiv > img[src*='iconClose']", ".cookiebar", ".cookiebar .cookiebar__button", ".cookiebar .cookiebar__link", ".cookiebar .cookiebar__form input[type=checkbox]", ".cookiebar .cookiebar__form input[type=checkbox]:checked:not(:disabled)", "cookie_consent_video=no", ".rc-notice:has(a[href$='/cookies-policy'])", ".rc-notice a[href$='/cookies-policy']", ".rc-notice:has(a[href$='/cookies-policy']) .rc-notice__actions button[data-slot=secondary]", ".rc-notice:has(a[href$='/cookies-policy']) .rc-notice__description button", ".rc-modal input[name=adData]", ".rc-modal input.rc-checkbox__input:checked", ".rc-modal:has(input[name=adData]) .rc-modal__footer button[data-slot=primary]", "third_party_ad_storage_consent%22%3Afalse", "privacy_settings=", ".gdpr-alert .gdpr-alert__close", ".gdpr-alert .gdpr-alert__overview", "#cookiePopUp", "#manageCookieSettingsPopUp", "#cookiePopUp[data-control=cookies]", "#cookiePopUp [data-bs-target='#manageCookieSettingsPopUp']", "#decline-cookies-button", "cookie_consents=", "#cookie_container", "#cookie_container #accept_cookies_btn", ".cookie-overlay .cookie-consent .cc-controls", ".cookie-consent .cc-controls .custom-button.--secondary-option + .custom-button.--secondary-option", "rittalCC=1", ".cookie-banner-wrapper", ".cookie-banner-wrapper .cookie-banner", ".cookie-banner-wrapper button.btn-secondary-lg", "RBXcb", '.MuiDrawer-paperAnchorBottom a[href*="/info/privacy"]', '.MuiDrawer-paperAnchorBottom [class*="-Grid-root-buttons"] button.MuiButton-outlined', "GoogleAnalytics=false", "#cookie-policy-info", 'div.cookie-btn-box > div[aria-label="Reject"]', '.cookie-policy-lightbox-bottom > div[aria-label="Save Settings"]', "#gdpr-notification", "#gdpr-notification #banner-link-preferences", "#banner-link-preferences", "#cookie-modal .cookie-modal-content", "#cookie-modal-input:checked", "#cookie-modal-input", "#cookie-modal-dismiss", "gdpr_accepts_cookies=false", "[role=dialog] form #cookie_consent_accept_all_button", "[role=dialog] form:has(#cookie_consent_accept_all_button) button[type=button]", "[role=dialog] form input#analytical", "[role=dialog] form input#analytical:checked", "[role=dialog] form:has(input#analytical) button[type=submit]:not(#cookie_consent_accept_all_button)", "#m-cookienotice", "#manage-cookies", "#accept-selected", ".cookie-consent .cookie-close", ".cookie-consent.is-show .cookie-close", "cookie=true", "rspb-cookie-banner", "rspb-cookie-banner .buttons button.btn-light", 'rspb-cookie-banner [data-testid="options"]', 'rspb-cookie-banner button[label="Save settings"]', "RSPB.AllowMarketingCookies=No", ".cookies-banner-shown", "#ConsentBar", '#ConsentBar [data-js--cookie="cookies-accept-functional"]', "#gdprconsent.gdprcontainer", "#gdprconsent .gdprbutton a", "gdpr-consent=0", "script[src*='consent.cookiebot.com/uc.js']", "#cookiebanner #DeclineAll", "statistics:false%2Cmarketing:false", "#cookie-container #cookie-confirm-btn", "cookies-confirm=true", "#cookie-popup-with-overlay", "#cookie-popup-with-overlay [data-ref='cookie.no-thanks']", "RY_COOKIE_CONSENT", "div.cookie-bar", ".cookie:has(.js-cookie-close)", "html.show-cookie .cookie .js-cookie-close", "cookieAgree=1", "#cookie-window", "#cookie-message", "#cookie-window #cookie-preview-accept", "#cookie-window, #cookie-message", "#popup-notice", "#popup-notice .popup__close", "saturdaysnyc-notice=1", "#kakrutan", "#kakrutan scb-button[value='inget']", '.cc-window.cc-type-info[aria-describedby="cookieconsent:desc"] .cc-dismiss', "body.cookie-overlay", "body > div#__next > div#app > div:nth-child(7):not([id])", "xpath///div[contains(@class, 'MuiBackdrop-root')][.//h1[contains(., 'Your Privacy Matters to Us')]]", "xpath///div[contains(@class, 'MuiBackdrop-root')][.//h1[contains(., 'Your Privacy Matters to Us')]]//button[normalize-space(.)='Reject Optional']", "xpath///div[contains(@class, 'MuiBackdrop-root')][.//h1[contains(., 'Your Privacy Matters to Us')]]//button[contains(normalize-space(.), 'MANAGE PREFERENCES')]", "xpath///div[contains(@class, 'MuiBackdrop-root')][.//h1[contains(., 'Manage Preferences')]]//button[normalize-space(.)='Save Preferences']", '"functional":false', '"performance":false', '"personalization":false', ".js-cookie-modal", '.js-cookie-modal [data-cookie-button-type="rejectCookies"]', ".js-cookie-modal.show-modal", '.js-cookie-modal [data-cookie-button-type="configureCookies"]', "useMarketingCookies=false", '[data-qa="GDPRBanner-Container"]', '[data-qa="GDPRBanner-Container"] [data-qa="GDPRBanner-Button"]', "#panelAsk", "#panelAsk #btnAcceptAllCookies", "#content-alert-cookie", "#content-alert-cookie #truste-consent-required2", "cookie_pi_accept=permit 1", "cookie_pi_accept=permit 1,2", "#cookieContainer #cookies", "#cookieContainer #cookies .cookies_inner button[onclick*='Rechazar']", "%7E%7Cparcial", "#cookie-info", "#cookie-info .button-row button.btn-secondary", "#cookie-info #no-ad-cookies", "#cookie-info form button.save-button", ".sw-cookie=1", ".l-cookie.js-cookie", ".l-cookie.js-cookie button", ".l-cookie.js-cookie.is-show", ".cookie-consent[data-cookieconsent], .gdpr-tab", ".cookie-consent[data-cookieconsent], .gdpr-tab #gdpr_tab_content", "__hs_opt_out=", "__hs_opt_out=yes", '[class*="cookie-consent-required"]', 'html[data-cookie-consent-required] [data-testid="accept-all-cookie-button"]', '[data-testid="accept-all-cookie-button"]', "body[data-scroll-locked]", '[data-testid="customize-cookie-button"]', '[data-testid="confirm-settings-cookie-button"]', '[data-testid="cookie-banner"] button:has(+ button)', 'dialog[open]:has(input[name^="cookiesetting."])', 'dialog[open] input[name^="cookiesetting."]:checked:not([disabled])', 'dialog[open]:has(input[name^="cookiesetting."]) > div > div.p-4 > button', "body > div[class*=_shein_privacy]", "body > div[class*=_shein_privacy] > div:nth-of-type(2) > div:nth-of-type(4) > div:nth-of-type(3)", ".cmp-cookies-notification", ".cmp-cookies-notification .cmp-cookies-notification__btn-cookie-accept", '#cookie-bar .cb-enable,[class^="app_gdpr--"] button', "#pnlCookieAcceptance", "#pnlCookieAcceptance #divCookiesManage", 'aside[x-data="cookieConsent"]', '#shopify-pc__banner, aside[x-data="cookieConsent"] > div.opacity-100', "#popinCookies .panelSimple", "#cookieWrapper #popinCookies", "#popinCookies #disagree_button, #cookieWrapper #popinCookies > [onclick*='refuse_all']", "#popinCookies.fadeInDown #disagree_button, #cookieWrapper #popinCookies > [onclick*='refuse_all']", "rgpd_cookies=1,0,0", "xpath///body/div[span/a[contains(@href, 'cookie-policy')]]/button", "xpath///body/div[span/a[contains(@href, 'cookie-policy')]]/button[normalize-space(.)='Endast n\xF6dv\xE4ndiga']", "cookieconsent_dismissed=dismissed", "custom-cookie-consent#cookieBubble.banner-show", "custom-cookie-consent#cookieBubble #configureBtn", "custom-cookie-consent#cookieBubble.banner-show #configureBtn", ".custom-cookie-modal #acceptNecessaryBtn", '[data-testid="cookie-wall"]', '[data-testid="cookie-wall"] .cookie-bar__footer button[data-id="0"]', "acceptCookies=0", "skv-cookie-banner", "SKVCookieAllow=strictlyNecessary", "#cookieBannerContent", "[data-tracking-element-id=cookie_banner_essential_only]", "[data-test-id=cookie-consent-banner]", "[data-test-id=cookie-consent-banner] #accept-necessary-cookies-button", "cookie-accept-necessary=accepted", "#app > div:has(> div:last-child > button:first-child + button:last-child)", "#app > div:not(:has(input[role=switch])) > div:last-child > button:last-child:not(:first-child)", "#app > div:has(input[role=switch]) > div:last-child > button:first-child", "#app > div:has(input[role=switch]) input[role=switch][aria-checked=true]:not([disabled])", "_s.cookie_consent=marketing=0:analytics=0", ".sm-alert[data-cookie^='cookie.disclaimer']", ".sm-alert.is-visible[data-cookie^='cookie.disclaimer'] .sm-alert__button", "disclaimer=closed", ".m-gdpr-banner", ".o-overlay--gdpr", ".m-gdpr-banner .js-m-gdpr-banner__button-manage", ".m-gdpr-banner.state-m-gdpr-banner--visible", ".m-gdpr-settings .js-m-gdpr-settings__button-custom", ".m-gdpr-settings input:checked:not(:disabled)", "#cookiesModal #cookiesModalCloseBtn", "#cookiesModalCloseBtn", "app-cookie-consent", "app-cookie-consent button.underline", "app-cookie-consent ui-switch button.switch", "app-cookie-consent ui-switch button.switch.checked:not(.disabled)", "app-cookie-consent app-btn button", "#cookie-consent-message", "#cookie-consent-message #cookie-consent-reject", "#cookie-consent-reject", "cookieconsent_status=0", ".plug-gdpr-wrapper.js-gdpr-wrapper", ".plug-gdpr-wrapper.js-gdpr-wrapper .js-gdpr-button", ".plug-gdpr-wrapper--show.js-gdpr-wrapper", '[aria-describedby="cookieconsent:desc"], .cc_overlay_bg', "#js-cookie-notification", "div.fixed.inset-0:has(.lucide-cookie)", "xpath///div[contains(@class, 'inset-0')]//button[contains(., 'Refuser tous')]", ".consent__wrapper", ".consent", "button.consentSettings", "button#consentSubmit", 'dialog[class*="CookieModal-module"]', 'dialog[class*="CookieModal-module"] button[class*="__secondaryButton"]', '[role="dialog"] [class*="SettingsModal-module"]', '[role="dialog"] [class*="SettingsModal-module"] input[type="checkbox"]:checked', '[role="dialog"]:has([class*="SettingsModal-module"]) [class*="__footer"] button', "targTK=false", "#cookiesPermission", "#cookiesPermission #cookiesClose", "#banniere_cookies", "#banniere_cookies span[onclick]", "[data-test=cookie-consent-modal]", "[data-test=cookie-consent-modal] [data-test=cookie-consent-decline-all]", "[data-test=cookie-consent-modal] [data-test=cookie-consent-preferences]", "[data-test=cookie-consent-settings-decline-all]", "surfshark-consent-state=a:0|m:0", ".CookieConsent__root", ".CookieConsent__root .CookieConsent__modal", ".CookieConsent__root button[data-consent-action=reject-all], #CookieConsent_reject_all", '"improved_experience":false,"partners":false', "#cookie.alert--cookie", "#cookie.alert--cookie[sy-alert]", "#cookie.alert--cookie.alert--active", "#cookie.alert--cookie button.alert-close", "synergy.alert.cookie=true", ".cookie-bar .cookie-box", ".cookie-bar .cookie-box .cookie-bar-buttons", ".cookie-bar .cookie-box .refuse-all", ".cookie-bar .cookie-box .cookie-bar-close", "#cookieChoiceInfo", "#cookieChoiceInfo #cookieChoiceLearnMore", "[id$='_ManageCookiesContent'] input[name$='BtnConfirmChoices']", "[id$='_ManageCookiesContent'] input[type=checkbox]:checked", "analyticsCookieConsent=n", "#coiOverlay", "#coiOverlay #declineButton", "#coiOverlay.coi-visible", "ts_cookie_choice=", "%22consents_denied%22%3A%5B%22cookie_cat_functional", "#myModal.showmodal", "#myModal.showmodal .showmodal-close", 'div:has([href="cookiePrivacyLink"]) + div:has(> [role=button][aria-label])', 'div:has([href="cookiePrivacyLink"]) + div:has(> [role=button][aria-label]) > [role=button][aria-label]:nth-child(2)', "privacy_setting_detail=%7B%22adsChoice%22%3A%220", "[aria-label=consent-banner]", "xpath///button[contains(., 'Reject all')]", "interactionCount=1", "#tsla-reject-cookie", "tsla-cookie-consent=rejected", '[data-testid="cookies-banner"]', '[data-testid="cookies-banner"] .cookies-banner__close-icon-wrapper', '[data-testid="cookies-banner"] .cookies-banner__description', '[data-testid="desktop-close-button"]', '[data-testid="cookies-banner"] .d-none-desktop .cookies-banner__close-icon-wrapper', ".cookie-popups__container", ".cookie-popups__first-consent-popup", '[data-test-id="cookie-manage-settings-button"]', ".cookie-popups__preferences-popup", ".cookie-popups__advertising-toggle:checked", '[data-test-id="cookie-advertising-slider"]', '[data-test-id="cookie-save-preferences-button"]', "lts-cookie-consent=1", "#user-consent", "#user-consent .page.main button.manage", "#user-consent .page.main", "#user-consent .page.secondary", "#user-consent .page.secondary input[type=checkbox]:checked:not([disabled])", "#user-consent .page.secondary button.save", '"marketing":"denied"', "#consent-banner button[data-testid='close-button']", '[data-callback="accounts/hide-banner/CookiesBanner"]', '[data-callback="accounts/hide-banner/CookiesBanner"] button[class*="ManageLink"]', '[data-testid="modal-body-content"] [class*="IndividualBox"] input[type="checkbox"]', '[data-testid="modal-body-content"] [class*="IndividualBox"] input[type="checkbox"][aria-disabled="false"]:checked', '[data-testid="modal-body-content"] [class*="Footer"] button', "%22advertising%22%3Afalse", ".optanon-alert-box-wrapper", ".optanon-alert-box-wrapper .optanon-toggle-display", "#optanon-popup-wrapper", "#optanon-menu li.menu-item-on:is(.menu-item-performance,.menu-item-functional,.menu-item-advertising,.menu-item-social) button", "#optanon-popup-more-info-bar .optanon-status-editable .optanon-status-on label", "#optanon-popup-bottom .optanon-save-settings-button button", "%2C2%3A0%2C3%3A0%2C4%3A0", "[role=dialog][aria-labelledby=cookie-banner-title]", "xpath///div[@role='dialog'][@aria-labelledby='cookie-banner-title']//button[normalize-space()='Manage preferences']", "xpath///div[@role='dialog'][.//input[@id='consent-toggle-analytics']]//button[normalize-space()='Reject all']", "#consent-toggle-analytics:not(:checked)", "xpath///div[@role='dialog'][.//input[@id='consent-toggle-analytics']]//button[normalize-space()='Save preferences']", "#cookie-notification #accept-cookie", "#cookie-notification:not(.hide)", "cookies-accepted=true", "div[role=alertdialog][data-nosnippet][aria-describedby]", "div[role=alertdialog][data-nosnippet][aria-describedby] > div > div:last-child > div:last-child > button", "div[role=dialog][data-nosnippet][aria-labelledby] > div > div:last-of-type > div > div:nth-child(2) > button", '[data-test="cookie-banner"]', "[data-test-id=cookies_section]", "#CookieNotice", "#CookieNotice > div[onclick]", "Notice=yes", "#tkConsentmanager", "#tkConsentmanager .g-consentmanager__confirm-selection", "tkConsentWirtschaftlicherWerbeeinsatz=0", "#cookie-consent #cookie-consent-btn-customise", "#cookie-consent #cookie-consent-btn-accept-some", "#cookie-consent-btn-customise", "#consent-marketing:checked", "#consent-marketing", "#cookie-consent-btn-accept-some", "%22analytics%22%3Afalse%2C%22marketing%22%3Afalse", "#cookie-compliant", "#cookie-compliant #saveAndAcceptCookies", "#cookie-compliant #cookies-config", "#cookie-compliant #cookie_b:checked", "#cookie-compliant #cookie_b", "#cookie-compliant #cookie_c:checked", "#cookie-compliant #cookie_c", "ALLOW_COOKIES=a", "ALLOW_COOKIES=a-", "xpath///div[contains(@class, 'fixed') and contains(., 'Cookie\u3092\u30D6\u30ED\u30C3\u30AF') and .//button[normalize-space()='Cookie\u306E\u8A2D\u5B9A']]", "xpath///div[contains(@class, 'fixed') and contains(., 'Cookie\u3092\u30D6\u30ED\u30C3\u30AF')]//button[normalize-space()='Cookie\u306E\u8A2D\u5B9A']", "xpath///button[normalize-space()='\u8A2D\u5B9A\u3092\u4FDD\u5B58\u3059\u308B']", ".optIn", ".optIn .js-optin-cookie-settings", 'label[for="TAB-02"]', 'label[for="TAB-02"] + .tab-content .toggleSwitch__circle', 'label[for="TAB-03"]', 'label[for="TAB-03"] + .tab-content .toggleSwitch__circle', ".js-modal-save-settings", "__performanceCookieFlag__=0", "__targetingCookieFlag__=0", "div:has(> button#accept-button):has(> button#decline-button)", "button#accept-button,button#decline-button", "button#decline-button", "div[role=dialog][aria-labelledby=privacy-notice-title]", "div[aria-labelledby=privacy-notice-title] button[role=switch][aria-checked=true]", "div[aria-labelledby=privacy-notice-title] button[role=switch][aria-checked=false]", "div[aria-labelledby=privacy-notice-title] button:not([role=switch]):last-child", "#consent-modal", "#privacy-settings-content", 'button[type="submit"]', "div.one-modal__action-footer-column--secondary > a", ".fixed.bottom-0:has([data-test=cookieBannerButton])", ".fixed.bottom-0 [data-test=cookieBannerButton]", "#cookie-modal-window button[name=save-cookies]", "#cookie-modal-window.show button[name=save-cookies]", "#cookie-modal-window #check-usability:checked", "#cookie-modal-window #check-usability", "#cookie-modal-window #check-statistics:checked", "#cookie-modal-window #check-statistics", "statistics", ".cm-Cookie.js-Cookie", ".cm-Cookie.js-Cookie .js-CookieBtn", '[class*="cookie-warning-wrapper"]', '[class*="cookie-warning-wrapper"] #notAllowCookiesButton', "#notAllowCookiesButton", "COOKIE_WARNING_REJECTED=true", "#pp .cookiesnoticev2", "#pp .cookiesnoticev2 #reject_all", '"analiticas": {"All":false', "#tse-consent", "#tse-consent .tse-more", "#tse-consent .tse-opts .tse-no", "#tse-consent .tse-t-ga:checked", "#tse-consent .tse-no", ".tvp-cookie-overlay", ".tvp-cookie-overlay .tvp-covl", ".tvp-covl .tvp-covl__sb", ".tvp-covl .tvp-covl__save", ".tvp-covl input.iabd:not(:checked)", ".tvp-covl input.iabd", "TVPtcs22=", '[data-a-target="consent-banner"],.ReactModalPortal:has([data-a-target=consent-modal-save])', '[data-a-target="consent-banner"]', '[data-a-target="consent-banner-accept"]', '[data-a-target="consent-banner"] button:not([data-a-target])', 'button[data-a-target="consent-banner-manage-preferences"]', "input[type=checkbox][data-a-target=tw-checkbox]:not([disabled])", "input[type=checkbox][data-a-target=tw-checkbox]:not(:checked):not([disabled])", "[data-a-target=consent-modal-save]", ".ReactModalPortal:has([data-a-target=consent-modal-save])", "#splash .subcolumns .subcr a[href='#']", "#splash .subcolumns", "#splash .subcr a[href='#']", 'huubShowInfo="false"', "#footer_tc_privacy", "#privacy__modal__close, #footer_tc_privacy #privacy__links__center", "#privacy__modal__close, #footer_tc_privacy", "#privacy__modal__close", "#footer_tc_privacy #privacy__links__center", "#privacy_options #options_tc_privacy_button", "UBI_PRIVACY_POLICY_VIEWED=true", "UBI_PRIVACY_ADS_OPTOUT=true", ".cookie-alert .cookies-manager__btn-open", ".cookie-alert button.cookies-manager__btn-open", ".cookies-manager__window", ".cookies-manager input.toggle:checked:not(:disabled)", ".cookies-manager__btn-save", "udgCookiesPreferences=set", ".toasterAlert .toaster", ".toasterAlert .toaster-content a[href*='/npp/OSPP-']", ".toasterAlert .toaster-close", "div[role='presentation']:has(p[data-sentry-source-file='CookieBanner.tsx'])", "p[data-sentry-source-file='CookieBanner.tsx']", "div[role='presentation']:has(p[data-sentry-source-file='CookieBanner.tsx']) button[data-sentry-element='UCButton']:nth-of-type(2)", "UCAcceptCookie=3", ".js-cookie-preferences", ".js-cookie-preferences .js-manage-cta", ".js-cookie-preferences.is-active", ".js-cookie-preferences .js-manage-panel button[type=submit]", "label[for=performace-no]", "label[for=targeted-advertising-no]", "advertisingCookies=false", "performanceAnalyticsCookies=false", "#__tealiumGDPRcpPrefs #consent-container", "#__tealiumGDPRcpPrefs #consent-container #consent-modal-settings", "#no-consent-popup-close-modal", "#cookieModal", '#cookieModal [onclick*="cookiesPolicy.selDefAndloadScript"]', "unive.it=1", "noiframe=checked", "socialpix=checked", "#acceptCookies", "#acceptCookies button", "app-cookie-consent > .cookie-banner", "app-cookie-consent .cookie-banner", "app-cookie-consent .cookie-banner > div > button:nth-of-type(2)", "#modal-consent-banner", "#modal-consent-banner #btn-edit-cookie-consent", "#cookie-consent-banner #btn-edit-cookie-consent", "#cookie-consent-dialog #btn-accept-some", "#consent-ad:checked", "#consent-ad", "#cookie-consent-dialog", ".ucb", ".ucb-banner", ".ucb-banner .ucb-btn-save", "#cookie-blur", "#cookie-notice #cookie_notice_manage", "COOKIES_ADVERTISING=0", "#js-pw-consent-wrapper bgl-modal[test-id='cookie-modal']", "#js-pw-consent-wrapper [data-ta-id='pw-consent-rejectAllButton']", "div.fixed:has(#modal-title) button.btn-cta", "div.fixed:has(#modal-title) p button", "label[for=cookies-functional]", "div.fixed:has(#cookies-functional:checked) button.btn-cta", "xsarusCookieSettings=disabled", ".alert-cookie #dismiss-cookie-banner-button", "vd_cookie_warning=1", ".gdpr_container", ".gdpr_modal_container", ".gdpr .gdpr_manage_cookies_button", ".gdpr .gdpr_container", ".gdpr_modal .gdpr_modal_button_reject", 'vg_consent={"functionality":false,"performance":false,"targeting":false}', ".CookieBanner", ".CookieBanner #Cookie_Selected", ".CookieBanner:not(.out) #Cookie_Selected", ".CookieBanner input[type=checkbox]:checked", "CookieConsent=required", "CookieConsent=required,", 'div.fixed.bottom-0:has(svg[data-icon="cookie-bite"])', 'div.fixed.bottom-0:has(svg[data-icon="cookie-bite"]) button', ".component-lightbox-gdpr", ".component-lightbox-gdpr .gdpr_cookie_settings_accept_cookies", ".gdpr_cookie_settings_accept_cookies__button-container__button.button--solid--secondary", ".gdpr_cookie_settings_cookie_settings", ".gdpr_cookie_settings_cookie_settings .component-toggle-button[toggle_selected=True]", ".gdpr_cookie_settings_cookie_settings__main-section__button-container__button", "_vinnova_gdpr=0", ".cookie-policy-modal", ".manage-cookies-modal", ".cookie-policy-modal .cookie-policy-details button.manage", ".manage-cookies-modal .manage-cookies-footer button.btn-outline-primary", ".dip-consent,.dip-consent-container", ".dip-consent-container", ".dip-consent-content", '.dip-consent-btn[tabindex="2"]', "dialog.cm-banner", "dialog.cm-banner .cm-actions", "dialog.cm-banner[open]", "dialog.cm-banner .cm-actions .cm-btn--outline", "xpath///dialog[contains(@class,'cm-banner')]//div[contains(@class,'cm-actions')]/button[normalize-space()='Weiger alles']", "cm_consent_v3=necessary|", "cookie-banner#cookie-banner-host", "C3%3A0", "C4%3A0", ".footer-signup:has(consent-scheduler > .cookie-consent-js:not(.hidden))", "consent-scheduler > .cookie-consent-js", ".footer-signup.active consent-scheduler > .cookie-consent-js:not(.hidden)", "#band-cookies", "#band-cookies-close", 'a[href="https://www.walmartcanada.ca/cookie-notice"]', 'button[data-dca-intent="open"][role="link"]', "xpath///button[contains(., 'Save settings') or contains(., 'Enregistrer les param\xE8tres')]", '.MuiDialog-root [role=dialog] img[src*="portalCKM-cookie-dialog"]', ".MuiDialog-root [role=dialog] button.MuiButton-outlined", ".MuiDialog-root [role=dialog] .MuiFormGroup-root button.MuiButton-outlined", ".MuiDialog-root [role=dialog] .MuiFormGroup-root input[type=checkbox]:checked:not(:disabled)", ".MuiDialog-root [role=dialog]", ".cookie-banner .btn-accept-selected", ".cookie-banner .toggle-switch[aria-pressed='true']", ".gdpr-popup", ".gdpr-popup #gdpr-popup__show-preferences", ".gdpr-popup.gdpr-popup--show", "#gdpr-popup__show-preferences", "#gdpr-popup__preferences button[data-gdpr=button][value=save]", "#gdpr-popup__preferences button[data-gdpr=switch][aria-checked=true]", "%22interactedWith%22:true", "%22marketing%22:%22revoked%22", "[role=dialog] [data-testid=view-preferences], .CookieNotice-Dialog [data-testid=Preferences]", "[role=dialog] [data-testid=view-preferences]", "[role=dialog] [data-testid=save-preferences]", ".CookieNotice-Dialog [data-testid=Preferences]", ".CookieNotice-Dialog [data-testid=UpdateAndClose]", "%22analytics%22:false%2C%22marketing%22:false", "[class^=cookie_wrapper]", ".shopify-section.cookies:has(.accept-cookies)", ".shopify-section.cookies .accept-cookies", "#cookiePolicy:has(#cookiePolicyClose)", "#cookiePolicy #cookiePolicyWrap #cookiePolicyClose", 'dialog[aria-labelledby="consent-title"]', "div.animate-slideFromLeft.fixed.bottom-0.z-50", 'dialog[aria-labelledby="consent-title"], div.animate-slideFromLeft.fixed.bottom-0.z-50', 'dialog[aria-labelledby="consent-title"] button.border-xe-primary-500, div.animate-slideFromLeft button.border-xe-primary-500', '[data-testid="performance-toggle"]', 'dialog[aria-labelledby="consent-title"] button.bg-xe-primary-500, div.animate-slideFromLeft button.bg-xe-primary-500', "xeConsentState={%22performance%22:false%2C%22marketing%22:false", '[data-role="cookies-modal"]', '[data-role="cookies-modal"] [data-opt-hydration="cookies-dialog-eu"]', '[data-role="cookies-modal"] [class*=experimentalButtons] button:nth-of-type(2)', '[data-role="cookies-modal"] [class*=container]', "[class*=cookiesAnnounce]", "[class*=cookiesAnnounce] [class*=announceText]", "form#consent-dialog button[data-testid=give-consent-button]", "form#consent-dialog", "form#consent-dialog button[name=read-more]", "form#consent-dialog fieldset", "form#consent-dialog fieldset input[type=checkbox]:checked:not([disabled])", "cookies_marketing_ok=false", "#cookieBar", "#cookieBar .js-cookies-btn", "#cookieBar [data-event=cookies-show-settings]", "#cookiesPopup .js-save-settings", "#cookiesPopup .js-switch.active:not(.is-required)", '"ad_storage":"denied"', "div.cookie.flex.flex-wrap", "xpath///div[contains(@class,'cookie')]//button[normalize-space()='Deny']", "#consent-page", "#consent-page button[value=reject]", "article:has(a[href*='shueisha.co.jp/privacy'])", "article:has(a[href*='shueisha.co.jp/privacy']) > div", "article:has(a[href*='shueisha.co.jp/privacy']) > div > div:last-child", "#colorbox.colorbox-cookie-consent", "#cboxOverlay.colorbox-cookie-consent", "#cookie-consent form.cookie-consent", "#colorbox input.cookie-consent-reject-all", "ConsentMarketing=False", ".cookies-box .cookies-btn-customize", ".manage-consent-preferences-dialog, .el-dialog[aria-label='Manage Consent Preferences']", ".manage-consent-preferences-dialog .van-checkbox__icon--checked, .el-dialog[aria-label='Manage Consent Preferences'] .el-checkbox.is-checked", ".manage-consent-preferences-dialog .van-dialog__confirm, .el-dialog[aria-label='Manage Consent Preferences'] .el-dialog__footer button", "yuntrackAccept=agree", "#cookie_modal_wrapper", "#cookie_modal_wrapper #cookie_modal_button_choose", "#consent-init", "#consent-init #consent-configure", "#consent-update #consent-configuration-save", "zinio-cookie-consent", ".cookies.main-class .customize-btn", ".cookies.main-class .cookies-container .customize-btn", ".cookies.main-class .cookies-hidden .el-switch", ".cookies-hidden .el-switch.is-checked:not(.is-disabled)", ".cookies.main-class .cookies-hidden .accept-btn", "validCookies=general--", "#rgpd-banner", "#rgpd-banner .gdrp-required", "cookie_consent_ads=off", "cookie_consent=1", "zz-cookie-banner", "user.consent=true", "consent.analytics=false"], r: [[1, "123inkt", 2, "", 22, [0], [{ e: 1 }], [{ check: "any", v: 2 }], [{ c: 1 }], [], {}], [1, "2gis", 2, "", 22, [3], [{ e: 3 }], [{ v: 3 }], [{ c: 4 }], [{ check: "none", timeout: 1e3, wv: 3 }], {}], [1, "abconcerts.be", 2, "", 22, [5], [{ e: 6 }], [{ v: 6 }], [{ if: { e: 7 }, then: [{ k: 7 }], else: [{ k: 8 }, { c: 9 }] }], [], { intermediate: false }], [1, "abebooks", 2, "", 22, [10], [{ e: 11 }], [{ v: 12 }], [{ c: 13 }, { check: "none", timeout: 2e3, optional: true, wv: 12 }], [{ cc: 14 }], {}], [1, "accept-cookie-box", 1, "", 22, [15], [{ e: 16 }], [{ v: 15 }], [{ h: 15 }], [], {}], [1, "acconsento", 2, "", 22, [17], [{ e: 18 }], [{ v: 18 }], [{ if: { e: 19 }, then: [{ c: 19 }], else: [{ c: 20 }] }], [{ cc: 21 }, { cc: 22 }], {}], [1, "acos-cms", 2, "", 22, [23], [{ e: 24 }], [{ v: 24 }], [{ c: 25 }, { check: "none", wv: 24 }], [{ cc: 26 }], { intermediate: false }], [1, "acris", 2, "", 22, [27], [{ e: 28 }], [{ v: 29 }], [{ check: "any", wv: 30 }, { wait: 500 }, { c: 30 }], [], {}], [1, "adaptivemedia", 2, "", 22, [31], [{ e: 32 }], [{ v: 33 }], [{ c: 34 }], [{ cc: 35 }], {}], [1, "admetricspro", 0, "", 22, [36, 37], [{ e: 38 }], [{ v: 39 }], [{ if: { e: 40 }, then: [{ all: true, optional: true, k: 41 }, { c: 42 }], else: [{ c: 43 }, { c: 44 }] }], [{ check: "none", timeout: 3e3, wv: 39 }], {}], [1, "adnuntius", 2, "", 22, [45], [{ e: 46 }], [{ v: 45 }], [{ c: 46 }], [{ check: "none", v: 45 }], {}], [1, "adopt", 2, "", 22, [47], [{ e: 48 }], [{ v: 47 }], [{ c: 49 }], [{ eval: "EVAL_ADOPT_TEST" }], {}], [1, "Adroll", 2, "", 22, [50], [{ e: 50 }], [{ v: 50 }], [{ c: 51 }], [{ negated: true, cc: 52 }], {}], [1, "adstate", 2, "", 22, [53], [{ exists: ["adstate-settings-component app-wrapper", "#cookie-pop-up-only-necessary"] }], [{ visible: ["adstate-settings-component app-wrapper", "app-cookie-pop-up"] }], [{ waitForThenClick: ["adstate-settings-component app-wrapper", "#cookie-pop-up-only-necessary"] }], [{ waitForVisible: ["adstate-settings-component app-wrapper", "app-cookie-pop-up"], check: "none", timeout: 2e3 }], {}], [1, "affinity.serif.com", 2, "", 22, [], [{ e: 54 }], [{ v: 55 }], [{ k: 56 }], [{ wait: 500 }, { cc: 57 }, { negated: true, cc: 58 }], {}], [1, "albelli-photobox", 0, "", 22, [59], [{ e: 60 }], [{ v: 60 }], [{ if: { v: 61 }, then: [{ c: 61 }], else: [{ if: { v: 62 }, then: [{ c: 62 }], else: [{ c: 63 }] }, { wv: 64 }, { all: true, optional: true, k: 65 }, { c: 64 }] }], [{ any: [{ cc: 66 }, { cc: 67 }] }], {}], [1, "alertamodal-cookies", 2, "", 22, [68], [{ e: 69 }], [{ v: 69 }], [{ c: 69 }], [{ cc: 70 }], {}], [1, "alertnotification-alert-bar", 1, "", 22, [71], [{ e: 71 }], [{ v: 71 }], [{ h: 71 }], [], {}], [1, "altamar-cookieconsent", 2, "", 22, [], [{ e: 72 }], [{ v: 72 }], [{ c: 72 }, { wv: 73 }, { if: { e: 74 }, then: [{ k: 75 }] }, { c: 76 }], [{ cc: 77 }], {}], [1, "amasty-gdpr", 0, "", 22, [], [{ e: 78 }], [{ v: 78 }], [{ timeout: 1e4, w: 79 }, { if: { v: 80 }, then: [{ c: 81 }], else: [{ k: 82 }, { wv: 83 }, { all: true, optional: true, k: 84 }, { retry: 3, retryInterval: 500, c: 85 }] }], [{ check: "none", timeout: 5e3, wv: 78 }], {}], [1, "amasty-gdpr-hyva", 0, "", 22, [], [{ e: 86 }], [{ v: 87 }, { negated: true, e: 88 }], [{ check: "none", timeout: 5e3, wv: 89 }, { if: { e: 90 }, then: [{ c: 90 }, { check: "none", timeout: 3e3, optional: true, wv: 87 }, { if: { v: 87 }, then: [{ k: 90 }] }], else: [{ c: 91 }, { timeout: 2e3, optional: true, wv: 89 }, { if: { v: 89 }, then: [], else: [{ k: 91 }] }, { wv: 89 }, { timeout: 5e3, wv: 92 }, { all: true, optional: true, k: 93 }, { c: 89 }, { check: "none", timeout: 1e4, optional: true, wv: 89 }, { check: "none", timeout: 1e3, optional: true, wv: 87 }, { if: { v: 87 }, then: [{ optional: true, k: 94 }] }] }], [{ any: [{ e: 88 }, { check: "none", timeout: 5e3, wv: 87 }] }], {}], [1, "amazon-hiring", 2, "", 22, [], [{ e: 95 }], [{ v: 95 }], [{ k: 96 }, { check: "none", timeout: 2e3, wv: 95 }, { if: { e: 97 }, then: [{ k: 97 }] }, { check: "none", timeout: 2e3, wv: 98 }], [], {}], [1, "amazon.com", 2, "", 22, [99], [{ e: 99 }], [{ check: "any", v: 100 }], [{ wv: 101 }, { wait: 5e3 }, { k: 101 }], [], {}], [1, "amex", 0, "", 22, [102], [{ e: 102 }], [{ v: 102 }], [{ c: 103 }], [], {}], [1, "anthropic", 2, "", 22, [], [{ e: 104 }], [{ v: 104 }], [{ c: 105 }], [{ cc: 106 }], {}], [1, "AppConsent", 2, "", 22, [107], [{ e: 108 }], [{ v: 107 }], [{ waitFor: ["iframe[srcdoc*='cdn.appconsent.io/tcf2-clear/']", ".button__refuseAll, .button__skip"] }, { if: { exists: ["iframe[srcdoc*='cdn.appconsent.io/tcf2-clear/']", ".button__refuseAll"] }, then: [{ click: ["iframe[srcdoc*='cdn.appconsent.io/tcf2-clear/']", ".button__refuseAll"] }], else: [{ click: ["iframe[srcdoc*='cdn.appconsent.io/tcf2-clear/']", ".button__skip"] }] }], [{ timeout: 1e3, check: "none", wv: 107 }], {}], [1, "AppConsent legacy", 2, "", 22, [109], [{ e: 110 }], [{ v: 109 }], [{ waitFor: ["iframe[srcdoc*='frame-root']", ".button__skip, .button__openPrivacyCenter"] }, { if: { exists: ["iframe[srcdoc*='frame-root']", ".button__skip"] }, then: [{ click: ["iframe[srcdoc*='frame-root']", ".button__skip"] }], else: [{ click: ["iframe[srcdoc*='frame-root']", ".button__openPrivacyCenter"] }, { waitForThenClick: ["iframe[srcdoc*='frame-root']", ".button__refuseAll"] }, { waitForThenClick: ["iframe[srcdoc*='frame-root']", ".button__save"] }] }], [{ timeout: 1e3, check: "none", wv: 109 }], {}], [1, "aquasana.com", 2, "", 22, [111], [{ v: 111 }], [{ v: 111 }], [{ if: { e: 112 }, then: [{ k: 112 }], else: [{ h: 111 }] }], [], {}], [1, "arbeitsagentur", 2, "", 22, [113], [{ e: 114 }], [{ e: 115 }], [{ waitForThenClick: ["bahf-cookie-disclaimer-dpl3", "#bahf-cookie-disclaimer-modal .ba-btn-contrast"] }], [{ cc: 116 }], {}], [1, "arto", 0, "", 22, [117], [{ e: 118 }], [{ v: 119 }], [{ c: 119 }], [{ cc: 120 }], {}], [1, "as-adventure", 2, "", 22, [121, 122, 123], [{ e: 124 }], [{ check: "any", v: 125 }], [{ all: true, retry: 10, retryInterval: 500, c: 124 }], [{ timeout: 2e3, check: "none", wv: 125 }], {}], [1, "asahi-interactive", 2, "", 22, [126], [{ e: 127 }], [{ v: 126 }], [{ c: 128 }], [{ cc: 129 }], {}], [1, "askas", 2, "", 22, [130], [{ e: 130 }, { e: 131 }, { e: 132 }], [{ v: 130 }], [{ if: { e: 133 }, then: [{ if: { check: "none", v: 133 }, then: [{ k: 134 }, { timeout: 2e3, optional: true, wv: 133 }] }, { k: 133 }], else: [{ if: { check: "none", v: 132 }, then: [{ k: 134 }, { timeout: 2e3, wv: 132 }] }, { all: true, optional: true, k: 135 }, { k: 132 }] }], [{ cc: 136 }, { cc: 137 }], {}], [1, "aspnet-cookieinfo", 0, "", 22, [138], [{ e: 139 }], [{ v: 138 }], [{ c: 139 }], [{ cc: 140 }], {}], [1, "astro-cookie-notice-fzbxxtek", 1, "", 22, [141], [{ e: 142 }], [{ v: 141 }], [{ h: 141 }], [], {}], [1, "asus", 2, "", 22, [143], [{ e: 144 }], [{ v: 144 }], [{ if: { e: 145 }, then: [{ k: 145 }], else: [{ c: 146 }, { c: 147 }] }], [], {}], [1, "atomstore", 0, "", 10, [148, 149], [{ e: 150 }], [{ check: "any", v: 151 }], [{ if: { e: 152 }, then: [{ c: 153 }], else: [{ if: { e: 154 }, then: [{ c: 155 }], else: [{ c: 156 }] }] }, { check: "none", wv: 151 }], [{ cc: 157 }], {}], [1, "autodoc", 2, "", 22, [158], [{ e: 159 }], [{ v: 159 }], [{ c: 159 }], [{ wait: 2e3 }, { cc: 160 }], {}], [1, "automattic-cmp-optout", 2, "", 22, [161], [{ e: 161 }], [{ v: 161 }], [{ k: 162 }, { all: true, c: 163 }, { k: 164 }], [], {}], [1, "autoscout24", 2, "", 22, [165], [{ e: 166 }], [{ v: 166 }], [{ c: 167 }, { timeout: 1e3, optional: true, c: 167 }], [{ wait: 500 }, { cc: 168 }], {}], [1, "avacy", 2, "", 22, [169, 170], [{ e: 171 }], [{ check: "any", v: 171 }], [{ if: { check: "any", v: 172 }, then: [{ c: 172 }], else: [{ if: { check: "any", v: 173 }, then: [{ k: 173 }], else: [{ if: { e: 174 }, then: [{ c: 174 }], else: [{ c: 175 }, { wv: 176 }, { if: { v: 177 }, then: [{ k: 177 }] }, { k: 176 }] }] }] }], [], {}], [1, "avada-cookie-consent", 2, "", 22, [178], [{ e: 179 }], [{ check: "any", v: 179 }], [{ c: 179 }, { any: [{ check: "none", timeout: 1e3, wv: 178 }, { c: 180 }] }], [{ cc: 181 }], {}], [1, "avada-privacy-bar", 2, "", 22, [182], [{ e: 183 }], [{ check: "any", v: 182 }], [{ if: { e: 184 }, then: [{ all: true, k: 184 }], else: [{ all: true, optional: true, k: 185 }, { all: true, k: 186 }] }], [{ check: "none", timeout: 1e3, wv: 182 }], {}], [1, "avada-privacy-bar-cosmetic", 1, "", 22, [182], [{ e: 186 }], [{ check: "any", v: 182 }], [{ h: 182 }], [], {}], [1, "avg-cookiebanner", 2, "", 22, [187], [{ e: 187 }], [{ negated: true, cc: 188 }, { v: 189 }], [{ if: { e: 190 }, then: [{ c: 190 }], else: [{ all: true, optional: true, k: 191 }, { if: { e: 192 }, then: [{ retry: 3, c: 192 }], else: [{ retry: 3, c: 193 }] }] }], [{ cc: 188 }], {}], [1, "avvio", 2, "", 22, [194], [{ e: 195 }], [{ check: "any", v: 196 }], [{ if: { v: 197 }, then: [{ c: 198 }] }, { wv: 199 }, { if: { e: 200 }, then: [{ all: true, k: 201 }] }, { k: 199 }, { check: "none", wv: 194 }], [{ cc: 202 }], {}], [1, "aws.amazon.com", 2, "", 22, [203, 204, 205, 206], [{ e: 203 }], [{ v: 203 }], [{ if: { e: 207 }, then: [{ k: 207 }], else: [{ k: 208 }, { w: 209 }, { all: true, optional: true, k: 210 }, { k: 211 }] }], [], {}], [1, "axeptio", 2, "", 22, [212], [{ e: 212 }], [{ any: [{ e: 213 }, { v: 214 }, { visible: [".axeptio_mount .needsclick", ".axeptio_widget"] }, { cc: 215 }] }], [{ if: { e: 216 }, then: [{ waitForVisible: [".axeptio_mount .needsclick", "button#axeptio_btn_dismiss,button.ax-discardButton,button#axeptio_btn_close"] }, { wait: 300 }, { if: { exists: [".axeptio_mount .needsclick", "button#axeptio_btn_dismiss,button.ax-discardButton"] }, then: [{ click: [".axeptio_mount .needsclick", "button#axeptio_btn_dismiss,button.ax-discardButton"] }], else: [{ click: [".axeptio_mount .needsclick", "button#axeptio_btn_close"] }] }], else: [{ wv: 217 }, { wait: 300 }, { if: { e: 218 }, then: [{ k: 218 }], else: [{ k: 219 }] }] }], [{ cc: 220 }], {}], [1, "axeptio-tcf", 2, "", 22, [221], [{ any: [{ e: 222 }, { exists: ["body > [style*='--ax-primary-color']", "[id^='axeptio_btn_']"] }] }], [{ visible: ["body > [style*='--ax-primary-color']", ".modal.show"] }], [{ waitForVisible: ["body > [style*='--ax-primary-color']", "#axeptio_btn_dismiss, .modal-continue a"] }, { if: { exists: ["body > [style*='--ax-primary-color']", "#axeptio_btn_dismiss"] }, then: [{ click: ["body > [style*='--ax-primary-color']", "#axeptio_btn_dismiss"] }], else: [{ click: ["body > [style*='--ax-primary-color']", ".modal-continue a"] }] }], [], {}], [1, "aylo-cookie-banner", 2, "", 22, [223], [{ e: 224 }, { e: 225 }], [{ v: 224 }, { any: [{ v: 226 }, { eval: "EVAL_AYLO_COOKIE_MANAGER_READY" }] }], [{ if: { v: 227 }, then: [{ c: 227 }], else: [{ if: { v: 228 }, then: [{ wait: 8e3 }, { c: 228 }], else: [{ c: 229 }] }] }, { optional: true, c: 230 }], [{ any: [{ cc: 231 }, { cc: 232 }] }], {}], [1, "b-cookie", 1, "", 22, [233], [{ e: 233 }], [{ v: 233 }], [{ h: 233 }], [], {}], [1, "baden-wuerttemberg.de", 2, "", 22, [234], [{ e: 234 }], [{ v: 234 }], [{ if: { e: 235 }, then: [{ k: 235 }], else: [{ all: true, optional: true, k: 236 }, { k: 237 }] }], [], {}], [1, "bancaditalia", 2, "", 22, [238], [{ e: 239 }], [{ v: 238 }], [{ c: 239 }], [{ cc: 240 }], {}], [1, "bathnes-cookie-banner", 2, "", 22, [241], [{ e: 241 }], [{ v: 242 }], [{ c: 242 }], [{ check: "none", timeout: 1e3, wv: 241 }], {}], [1, "bayern-stmas", 2, "", 22, [243], [{ e: 244 }], [{ v: 244 }], [{ if: { e: 245 }, then: [{ k: 246 }] }, { retry: 3, retryInterval: 500, c: 244 }, { check: "none", timeout: 2e3, optional: true, wv: 243 }], [{ check: "none", timeout: 2e3, wv: 243 }], {}], [1, "bbb.org", 0, "", 22, [247], [{ e: 248 }], [{ v: 249 }], [{ wv: 250 }, { wait: 500 }, { k: 250 }, { w: 251 }, { all: true, optional: true, k: 252 }, { k: 253 }, { c: 254 }], [], {}], [1, "bbva", 2, "", 22, [255], [{ e: 256 }], [{ v: 256 }], [{ c: 256 }, { check: "none", wv: 257 }], [{ cc: 258 }], {}], [1, "bdirect-booking-engine", 2, "", 22, [259], [{ e: 260 }], [{ v: 260 }], [{ retry: 2, c: 261 }], [{ cc: 262 }], { intermediate: false }], [1, "beetle-tracking", 2, "", 22, [263], [{ exists: ["beetle-tracking-consent-modal", ".consent"] }], [{ visible: ["beetle-tracking-consent-modal", ".consent__dialog-wrapper"] }], [{ waitForThenClick: ["beetle-tracking-consent-modal", ".consent__button-decline-all"], retry: 10, retryInterval: 500 }], [{ cc: 264 }], {}], [1, "betheme", 2, "", 22, [265], [{ e: 266 }], [{ v: 267 }], [{ c: 266 }], [{ cc: 268 }], {}], [1, "bigcommerce-consent-manager", 0, "", 22, [269], [{ v: 269 }, { eval: "EVAL_BIGCOMMERCE_CONSENT_MANAGER_DETECT" }], [{ check: "any", v: 270 }], [{ c: 271 }], [{ cc: 272 }], {}], [1, "bigcrunch", 2, "", 10, [273, 274], [{ exists: ["#bc-cmp-host, #bc-cmp-host-auto", "#bc-cmp-banner"] }], [{ visible: ["#bc-cmp-host, #bc-cmp-host-auto", '#bc-cmp-banner-container:not([style*="display: none"]) #bc-cmp-banner'] }], [{ waitForThenClick: ["#bc-cmp-host, #bc-cmp-host-auto", "#bc-cmp-banner [data-action=reject-all]"] }], [{ waitForVisible: ["#bc-cmp-host, #bc-cmp-host-auto", '#bc-cmp-banner-container:not([style*="display: none"]) #bc-cmp-banner'], timeout: 2e3, check: "none" }], {}], [1, "bigid", 2, "", 22, [275], [{ e: 275 }], [{ visible: ["#bigidcmp-banner-widget", "#cmp-accept-all"] }], [{ if: { exists: ["#bigidcmp-banner-widget", "#cmp-reject-all"] }, then: [{ waitForThenClick: ["#bigidcmp-banner-widget", "#cmp-reject-all"] }], else: [{ if: { exists: ["#bigidcmp-banner-widget", "#cmp-custom-permissions"] }, then: [{ waitForThenClick: ["#bigidcmp-banner-widget", "#cmp-custom-permissions"] }, { waitForVisible: ["#bigidcmp-banner-widget", "#cmp-submit"] }, { click: ["#bigidcmp-banner-widget", 'button[role="switch"]:not([disabled]) > span[class*="translate-x-[1.1rem]"]'], all: true, optional: true }, { waitForThenClick: ["#bigidcmp-banner-widget", "#cmp-submit"] }], else: [{ waitForThenClick: ["#bigidcmp-banner-widget", 'div[tabindex="0"]:has(> svg #times-solid)'] }] }] }], [{ cc: 276 }, { cc: 277 }], {}], [1, "bing.com", 2, "", 22, [278], [{ e: 279 }], [{ v: 279 }, { v: 280 }], [{ wait: 500 }, { c: 281 }], [{ cc: 282 }], {}], [1, "binogi", 2, "", 22, [283], [{ e: 284 }], [{ v: 283 }], [{ retry: 3, c: 284 }], [{ check: "none", v: 283 }], {}], [1, "bleau-cookiecontrol", 2, "", 22, [285], [{ e: 286 }], [{ v: 287 }], [{ c: 286 }], [{ check: "none", wv: 285 }, { cc: 288 }], {}], [1, "blocksy", 0, "", 10, [289], [{ e: 290 }], [{ v: 289 }], [{ c: 291 }], [{ cc: 292 }], { intermediate: false }], [1, "blogger-cookie-notice", 2, "", 22, [293], [{ e: 294 }], [{ v: 293 }], [{ c: 295 }], [{ cc: 296 }], {}], [1, "bluepark", 2, "", 22, [297], [{ e: 298 }], [{ v: 299 }], [{ c: 299 }], [{ check: "none", timeout: 3e3, wv: 300 }], {}], [1, "bnes-cc", 2, "", 22, [301], [{ e: 302 }], [{ v: 303 }], [{ if: { e: 304 }, then: [{ c: 304 }], else: [{ c: 302 }, { wv: 305 }, { if: { e: 306 }, then: [{ k: 307 }] }, { if: { e: 308 }, then: [{ k: 309 }] }, { k: 305 }] }], [{ cc: 310 }, { negated: true, cc: 311 }], {}], [1, "bold-cookie-consent", 0, "", 22, [312, 313], [{ e: 314 }], [{ if: { v: 313 }, then: [{ v: 313 }], else: [{ v: 312 }, { check: "any", v: 315 }] }], [{ if: { v: 313 }, then: [{ k: 316 }, { check: "none", timeout: 3e3, optional: true, wv: 313 }], else: [{ if: { check: "any", v: 317 }, then: [{ k: 317 }], else: [{ k: 318 }, { wv: 314 }, { all: true, optional: true, k: 319 }, { k: 320 }] }, { check: "none", timeout: 3e3, optional: true, wv: 312 }] }], [{ if: { e: 313 }, then: [{ check: "none", v: 313 }], else: [{ check: "none", v: 312 }] }], {}], [1, "borlabs", 2, "", 22, [321], [{ e: 322 }], [{ v: 323 }], [{ timeout: 3e3, optional: true, w: 324 }, { if: { e: 325 }, then: [{ k: 325 }], else: [{ if: { e: 326 }, then: [{ k: 326 }], else: [{ if: { e: 327 }, then: [{ k: 327 }], else: [{ if: { e: 328 }, then: [{ k: 328 }, { wv: 329 }, { all: true, optional: true, k: 330 }, { k: 331 }, { wait: 500 }] }] }] }] }], [{ eval: "EVAL_BORLABS_0" }], {}], [1, "bosch-dock-privacy-settings", 2, "", 22, [332], [{ e: 332 }], [{ visible: ["dock-privacy-settings", "#decline-all-modal-dialog button"] }], [{ waitForThenClick: ["dock-privacy-settings", "#decline-all-modal-dialog button"] }], [{ waitForVisible: ["dock-privacy-settings", "#decline-all-modal-dialog button"], timeout: 1e3, check: "none" }], {}], [1, "botble-cookie-consent", 2, "", 22, [333], [{ e: 334 }], [{ v: 335 }], [{ retry: 5, retryInterval: 500, c: 335 }], [{ check: "none", v: 335 }], {}], [1, "bouvet-cookiepanel", 2, "", 22, [336], [{ e: 336 }], [{ v: 336 }], [{ c: 337 }], [{ cc: 338 }], {}], [1, "brainsum-cookieconsent", 2, "", 22, [339], [{ e: 340 }], [{ v: 341 }], [{ if: { v: 342 }, then: [{ k: 342 }], else: [{ k: 343 }, { wv: 344 }, { all: true, optional: true, k: 345 }, { all: true, optional: true, k: 346 }, { k: 344 }] }], [{ check: "none", wv: 341 }, { cc: 288 }], {}], [1, "bsgdprcookies", 2, "", 22, [347], [{ e: 348 }], [{ v: 349 }], [{ c: 348 }, { wv: 350 }, { all: true, optional: true, k: 351 }, { k: 349 }, { check: "none", timeout: 2e3, optional: true, wv: 347 }], [{ cc: 352 }], {}], [1, "bswhealth", 0, "", 22, [353], [{ e: 353 }], [{ v: 354 }], [{ c: 355 }, { wv: 356 }, { all: true, optional: true, c: 357 }, { c: 354 }], [{ cc: 358 }], {}], [1, "bullionbypost", 2, "", 22, [], [{ e: 359 }, { e: 360 }], [{ v: 359 }], [{ if: { e: 361 }, then: [{ k: 362 }] }, { c: 363 }, { if: { e: 364 }, then: [{ wait: 1e3 }, { k: 363 }] }, { if: { e: 364 }, then: [{ wait: 1e3 }, { k: 363 }] }, { if: { e: 364 }, then: [{ wait: 1e3 }, { k: 363 }] }, { if: { e: 364 }, then: [{ wait: 1e3 }, { k: 363 }] }, { if: { e: 364 }, then: [{ wait: 1e3 }, { k: 363 }] }], [{ cc: 365 }, { cc: 366 }], {}], [1, "bundesarchiv-cookiebanner", 2, "", 22, [367, 368], [{ e: 369 }], [{ v: 369 }], [{ if: { e: 370 }, then: [{ all: true, k: 371 }] }, { c: 369 }], [{ timeout: 2e3, check: "none", wv: 369 }], {}], [1, "bundesregierung.de", 2, "", 22, [372], [{ e: 372 }], [{ v: 373 }], [{ wait: 500 }, { c: 374 }], [{ cc: 375 }], {}], [2, "burpee.com", 2, "", 22, [376, 377], [{ e: 377 }, { negated: true, e: 378 }], [{ v: 379 }], [{ if: { e: 380 }, then: [{ c: 380 }], else: [{ if: { e: 381 }, then: [{ c: 381 }], else: [{ if: { e: 382 }, then: [{ c: 382 }, { all: true, k: 383 }, { k: 384 }], else: [{ h: 385 }, { removeClass: "cookie-notification-active", selector: "body", optional: true }] }] }] }], [], {}], [1, "bwr-cookie", 2, "", 22, [386], [{ e: 387 }], [{ v: 388 }], [{ all: true, optional: true, k: 389 }, { c: 387 }, { check: "none", timeout: 2e3, optional: true, wv: 386 }], [{ cc: 390 }], {}], [1, "bynder", 2, "", 22, [391], [{ e: 392 }], [{ v: 392 }], [{ c: 392 }], [{ cc: 393 }, { cc: 394 }], {}], [1, "c-banner-allow-cookies", 0, "", 10, [395], [{ e: 396 }], [{ v: 395 }], [{ c: 396 }, { check: "none", wv: 395 }], [{ cc: 397 }], {}], [1, "c-cookie-message", 1, "", 22, [398], [{ e: 399 }], [{ v: 399 }], [{ h: 398 }], [], {}], [1, "c15t", 0, "", 22, [400], [{ e: 401 }], [{ v: 401 }], [{ if: { e: 402 }, then: [{ k: 402 }], else: [{ k: 403 }, { timeout: 2e3, wv: 404 }, { all: true, optional: true, k: 405 }, { k: 404 }] }], [{ any: [{ cc: 406 }, { cc: 407 }] }, { negated: true, cc: 408 }, { negated: true, cc: 409 }], {}], [1, "casasrurales", 0, "", 22, [410], [{ e: 411 }], [{ v: 412 }], [{ c: 411 }, { c: 413 }, { check: "none", timeout: 3e3, optional: true, wv: 410 }], [{ cc: 414 }], {}], [1, "casino-guru", 1, "", 22, [415], [{ e: 416 }], [{ v: 417 }], [{ h: 415 }], [], {}], [1, "cassie", 0, "", 10, [418], [{ e: 419 }], [{ v: 420 }], [{ c: 421 }], [], {}], [1, "cc_banner", 1, "", 22, [422], [{ e: 422 }], [{ v: 423 }], [{ h: 422 }], [], {}], [1, "cc-banner-springer", 2, "", 22, [424], [{ e: 424 }], [{ v: 424 }], [{ if: { e: 425 }, then: [{ k: 425 }], else: [{ c: 426 }, { w: 427 }, { all: true, optional: true, k: 428 }, { if: { e: 429 }, then: [{ k: 429 }], else: [{ k: 430 }] }] }], [{ eval: "EVAL_CC_BANNER2_0" }], {}], [1, "ccm19", 2, "", 22, [431], [{ any: [{ e: 432 }, { e: 433 }] }], [{ v: 431 }], [{ if: { e: 434 }, then: [{ c: 434 }], else: [{ c: 435 }, { c: 436 }] }], [{ timeout: 2e3, check: "none", wv: 431 }], {}], [1, "ccoo", 2, "", 22, [437], [{ e: 438 }], [{ v: 439 }], [{ c: 440 }, { wv: 441 }, { all: true, optional: true, k: 442 }, { c: 438 }], [{ cc: 443 }], {}], [1, "ceneo", 2, "", 22, [444], [{ e: 445 }], [{ v: 445 }], [{ c: 445 }], [{ timeout: 1e3, check: "none", wv: 444 }], {}], [1, "cf-gdpr-cookie-consent", 2, "", 22, [446], [{ e: 447 }], [{ v: 447 }], [{ c: 447 }, { wv: 448 }, { if: { e: 449 }, then: [{ all: true, k: 449 }] }, { k: 448 }, { check: "none", wv: 446 }], [{ cc: 450 }, { cc: 451 }, { cc: 452 }], {}], [1, "check24", 2, "", 22, [453], [{ e: 454 }], [{ v: 455 }], [{ c: 454 }], [{ check: "none", v: 453 }], {}], [1, "check24-partnerprogramm-de", 2, "", 22, [456], [{ e: 457 }], [{ check: "any", v: 457 }], [{ c: 458 }], [], {}], [1, "chefcookie", 2, "", 22, [459], [{ e: 460 }], [{ v: 461 }], [{ c: 462 }], [{ cc: 463 }], {}], [1, "ciaopeople.it", 2, "", 22, [464], [{ e: 465 }], [{ v: 466 }], [{ c: 467 }], [{ check: "none", timeout: 5e3, wv: 466 }], {}], [1, "cinemaplus", 2, "", 22, [468], [{ e: 469 }], [{ v: 469 }], [{ c: 469 }], [{ check: "none", v: 469 }], {}], [1, "civic-cookie-control", 2, "", 22, [470], [{ e: 471 }], [{ v: 472 }, { v: 471 }], [{ if: { v: 473 }, then: [{ c: 473 }], else: [{ if: { e: 474 }, then: [{ c: 474 }, { wv: 475 }] }, { if: { v: 473 }, then: [{ c: 473 }], else: [{ if: { e: 476 }, then: [{ c: 476 }], else: [{ c: 477 }] }] }] }], [], {}], [1, "ckies", 2, "", 22, [478], [{ e: 479 }], [{ v: 478 }], [{ c: 479 }], [{ cc: 480 }], {}], [1, "cksmodal", 2, "", 22, [481], [{ e: 482 }], [{ v: 483 }], [{ c: 484 }, { check: "none", wv: 481 }], [{ cc: 485 }], {}], [1, "click.io", 2, "", 22, [486], [{ e: 486 }], [{ v: 486 }], [{ w: 487 }, { wait: 500 }, { k: 487 }, { w: 488 }, { all: true, k: 488 }, { k: 489 }], [{ cc: 490 }], {}], [1, "clinch", 2, "", 10, [491], [{ e: 491 }], [{ v: 491 }], [{ if: { e: 492 }, then: [{ k: 492 }], else: [{ k: 493 }, { all: true, optional: true, k: 494 }, { k: 495 }] }], [{ cc: 496 }], { intermediate: false }], [1, "close-cookie-notice-snippet", 1, "", 22, [497], [{ e: 498 }], [{ v: 497 }], [{ h: 497 }], [], {}], [1, "cloudflare-zaraz", 2, "", 22, [499], [{ exists: [".cf_modal_container", ".cf_consent-buttons"] }], [{ visible: [".cf_modal_container", ".cf_consent-buttons"] }], [{ waitForThenClick: [".cf_modal_container", ".cf_consent-buttons #cf_consent-buttons__reject-all"] }], [], {}], [1, "cm-cookie-manager", 2, "", 22, [500, 501], [{ e: 502 }, { e: 503 }], [{ v: 503 }], [{ if: { v: 504 }, then: [{ k: 504 }] }, { wv: 502 }, { if: { e: 505 }, then: [{ all: true, k: 505 }] }, { k: 502 }], [{ check: "none", v: 503 }], {}], [1, "codalt-cookie-consent", 2, "", 22, [506], [{ e: 507 }], [{ v: 508 }], [{ c: 509 }], [], {}], [1, "code3-cookies-consent", 2, "", 22, [510], [{ e: 510 }, { e: 511 }], [{ v: 510 }], [{ if: { v: 512 }, then: [{ k: 512 }], else: [{ k: 513 }] }], [{ cc: 514 }, { cc: 515 }], {}], [1, "coki61", 2, "", 22, [], [{ e: 516 }], [{ v: 517 }], [{ c: 516 }], [{ cc: 518 }], {}], [1, "columbia-privacy-notice", 1, "", 22, [519], [{ e: 520 }], [{ v: 519 }], [{ h: 519 }], [{ check: "none", v: 519 }], {}], [1, "com-consent-layer", 2, "", 22, [521], [{ exists: ["com-consent-layer", "dialog"] }], [{ visible: ["com-consent-layer", "dialog[open]"] }], [{ waitForThenClick: ["com-consent-layer", "#cmpDenyAll"] }], [{ waitForVisible: ["com-consent-layer", "dialog[open]"], timeout: 1e3, check: "none" }], {}], [1, "Complianz banner", 2, "", 22, [522], [{ e: 523 }], [{ check: "any", v: 523 }], [{ c: 524 }], [{ cc: 525 }], {}], [1, "Complianz categories", 2, "", 22, [526], [{ e: 526 }], [{ v: 526 }], [{ if: { e: 527 }, then: [{ k: 528 }], else: [{ all: true, optional: true, k: 529 }, { k: 530 }] }], [], {}], [1, "Complianz notice", 1, "", 22, [531], [{ e: 532 }], [{ check: "any", v: 533 }], [{ if: { e: 534 }, then: [{ k: 534 }], else: [{ h: 535 }, { optional: true, h: 536 }] }], [], {}], [1, "Complianz opt-out", 2, "", 22, [537], [{ e: 537 }], [{ v: 537 }], [{ timeout: 2e3, optional: true, w: 538 }, { if: { e: 534 }, then: [{ k: 534 }], else: [{ if: { e: 539 }, then: [{ k: 539 }, { wv: 540 }, { optional: true, k: 541 }, { k: 540 }] }] }], [], {}], [1, "Complianz optin", 2, "", 22, [542], [{ e: 542 }], [{ v: 543 }], [{ if: { v: 534 }, then: [{ k: 534 }], else: [{ if: { v: 544 }, then: [{ c: 544 }, { wv: 545 }, { all: true, optional: true, k: 546 }, { k: 547 }], else: [{ if: { e: 528 }, then: [{ k: 528 }], else: [{ k: 539 }, { c: 541 }, { c: 540 }] }] }] }, { optional: true, h: 548 }], [], {}], [1, "compricer", 2, "", 22, [549, 550], [{ e: 551 }, { e: 552 }], [{ v: 553 }], [{ c: 553 }, { wv: 551 }, { all: true, optional: true, k: 554 }, { k: 551 }, { check: "none", optional: true, wv: 551 }], [{ check: "none", timeout: 2e3, wv: 553 }, { negated: true, e: 554 }], {}], [1, "comundus-cookie-banner", 2, "", 22, [555], [{ e: 556 }], [{ v: 555 }], [{ if: { v: 557 }, then: [{ k: 557 }, { timeout: 2e3, wv: 558 }] }, { all: true, optional: true, k: 559 }, { c: 558 }], [], {}], [1, "concept3d", 2, "", 22, [560], [{ e: 561 }], [{ v: 561 }], [{ c: 561 }, { wv: 562 }, { all: true, optional: true, k: 563 }, { c: 562 }], [{ cc: 564 }], {}], [1, "conextra", 2, "", 22, [565, 566], [{ e: 567 }], [{ v: 567 }], [{ if: { e: 568 }, then: [{ c: 568 }], else: [{ all: true, optional: true, k: 569 }, { c: 570 }] }], [{ timeout: 1e3, check: "none", wv: 567 }], {}], [1, "consent-backdrop", 2, "", 22, [571], [{ e: 572 }], [{ v: 571 }], [{ all: true, optional: true, k: 573 }, { c: 574 }], [{ cc: 575 }], {}], [1, "consent-flo", 2, "", 22, [576], [{ e: 577 }], [{ v: 578 }], [{ if: { e: 579 }, then: [{ c: 579 }], else: [{ c: 580 }, { wv: 581 }, { all: true, optional: true, k: 582 }, { c: 581 }] }], [{ e: 583 }], {}], [1, "consent-studio", 0, "", 10, [584, 585], [{ e: 586 }], [{ v: 586 }], [{ if: { v: 587 }, then: [{ k: 587 }], else: [{ if: { check: "any", v: 588 }, then: [{ k: 588 }], else: [{ if: { e: 589 }, then: [{ all: true, k: 589 }] }, { c: 590 }] }] }, { check: "none", wv: 586 }], [{ wait: 500 }, { cc: 591 }], {}], [1, "consentaur", 2, "", 22, [592], [{ e: 592 }], [{ v: 593 }], [{ c: 593 }], [{ cc: 594 }], {}], [1, "consentmagic", 0, "", 10, [], [{ v: 595 }], [{ v: 595 }], [{ if: { e: 596 }, then: [{ retry: 5, retryInterval: 500, c: 596 }], else: [{ c: 597 }, { wv: 598 }, { all: true, optional: true, k: 599 }, { c: 598 }] }], [{ cc: 600 }], {}], [1, "consentmanager-ncmp", 2, "", 22, [601], [{ e: 602 }], [{ check: "any", v: 602 }], [{ wait: 500 }, { eval: "EVAL_CONSENTMANAGER_NCMP_REJECT" }], [{ cc: 603 }], {}], [1, "consentmo", 2, "", 22, [604], [{ exists: ["csm-cookie-consent", ".csm-wrapper"] }], [{ visible: ["csm-cookie-consent", ".csm-wrapper button"] }], [{ if: { exists: ["csm-cookie-consent", ".csm-wrapper .cc-deny, .csm-wrapper [data-csm-button-type='reject']"] }, then: [{ waitForThenClick: ["csm-cookie-consent", ".csm-wrapper .cc-deny, .csm-wrapper [data-csm-button-type='reject']"] }], else: [{ if: { exists: ["csm-cookie-consent", ".csm-wrapper [data-csm-button-type='do_not_sell']"] }, then: [{ waitForThenClick: ["csm-cookie-consent", ".csm-wrapper [data-csm-button-type='do_not_sell']"] }, { waitForThenClick: ["csm-cookie-consent", ".csm-wrapper [data-csm-button-type='dismiss']"], timeout: 2e3, optional: true }], else: [{ waitForThenClick: ["csm-cookie-consent", ".csm-wrapper .cc-settings, .csm-wrapper [data-csm-button-type='preferences']"] }, { waitForVisible: ["csm-cookie-consent", ".csm-wrapper .cc-deny, .csm-wrapper [data-csm-button-type='accept_selected'], .csm-wrapper [data-preferences-csm-button-type='accept_selected']"] }, { if: { exists: ["csm-cookie-consent", ".csm-wrapper .cc-deny"] }, then: [{ click: ["csm-cookie-consent", ".csm-wrapper .cc-deny"] }], else: [{ click: ["csm-cookie-consent", ".csm-wrapper [data-csm-button-type='accept_selected'], .csm-wrapper [data-preferences-csm-button-type='accept_selected']"] }] }] }] }], [{ waitForVisible: ["csm-cookie-consent", ".csm-wrapper button"], timeout: 1e3, check: "none" }], {}], [1, "Cookie Information Banner", 2, "", 22, [605], [{ e: 605 }], [{ v: 605 }], [{ eval: "EVAL_COOKIEINFORMATION_0" }, { wait: 1e3 }, { if: { v: 606 }, then: [{ h: 605 }] }], [{ cc: 607 }], {}], [1, "cookie-accordion-settings", 2, "", 22, [608], [{ e: 609 }, { e: 610 }], [{ v: 611 }], [{ c: 611 }, { wv: 609 }, { if: { v: 612 }, then: [{ k: 612 }], else: [{ all: true, optional: true, k: 613 }, { k: 609 }] }, { check: "none", timeout: 3e3, wv: 610 }], [], {}], [1, "cookie-agree-banner", 1, "", 22, [614], [{ e: 615 }], [{ v: 615 }], [{ h: 614 }], [], {}], [1, "cookie-banner-element", 2, "", 22, [616], [{ e: 616 }], [{ visible: ["cookie-banner-element#ckb", '#ckb[role="dialog"]'] }], [{ waitForThenClick: ["cookie-banner-element#ckb", "#ckn"] }], [], {}], [1, "cookie-bar-eu", 2, "", 22, [617, 618], [{ e: 619 }, { e: 620 }], [{ v: 617 }], [{ if: { v: 621 }, then: [{ k: 621 }, { timeout: 2e3, optional: true, wv: 622 }] }, { if: { e: 623 }, then: [{ k: 624 }] }, { if: { e: 625 }, then: [{ k: 626 }] }, { k: 622 }], [{ cc: 627 }, { cc: 628 }], {}], [1, "cookie-bar-frame", 2, "", 22, [617], [{ e: 629 }], [{ v: 630 }], [{ c: 629 }], [{ cc: 631 }], {}], [1, "cookie-box-notice", 2, "", 22, [], [{ e: 632 }], [{ v: 632 }, { negated: true, e: 633 }], [{ c: 632 }], [{ any: [{ e: 633 }, { check: "none", timeout: 2e3, wv: 632 }] }], {}], [1, "cookie-consent-root", 2, "", 22, [634], [{ e: 635 }], [{ v: 634 }], [{ c: 636 }, { wv: 637 }, { if: { e: 638 }, then: [{ k: 639 }] }, { if: { e: 640 }, then: [{ k: 641 }] }, { k: 637 }], [{ cc: 642 }], {}], [1, "cookie-consent-spice", 0, "", 10, [643, 644], [{ e: 644 }], [{ v: 644 }], [{ c: 645 }], [], {}], [1, "cookie-info-banner", 2, "", 22, [646], [{ e: 647 }], [{ v: 648 }], [{ c: 647 }], [{ cc: 649 }, { negated: true, cc: 650 }], {}], [1, "cookie-law-info", 2, "", 22, [651], [{ e: 652 }, { eval: "EVAL_COOKIE_LAW_INFO_DETECT" }], [{ v: 652 }], [{ h: 651 }, { eval: "EVAL_COOKIE_LAW_INFO_0" }], [{ negated: true, cc: 653 }], {}], [1, "cookie-manager-popup", 0, "", 10, [654], [{ e: 655 }], [{ v: 377 }], [{ if: { e: 656 }, then: [{ k: 656 }], else: [{ c: 654 }, { wv: 657 }, { all: true, optional: true, k: 658 }, { k: 659 }] }], [{ eval: "EVAL_COOKIE_MANAGER_POPUP_0" }], { intermediate: false }], [1, "cookie-notice-checkboxes", 2, "", 22, [660], [{ e: 661 }], [{ v: 661 }], [{ if: { e: 662 }, then: [{ c: 662 }], else: [{ if: { e: 663 }, then: [{ all: true, k: 663 }] }, { c: 664 }] }], [{ check: "none", v: 664 }], {}], [1, "cookie-popup-close-cookies", 2, "", 22, [665], [{ e: 666 }, { e: 667 }], [{ e: 668 }], [{ eval: "EVAL_COOKIE_POPUP_CLOSE" }, { timeout: 2e3, w: 669 }], [{ negated: true, cc: 670 }], {}], [1, "cookie-popup-cookieno", 2, "", 22, [671], [{ e: 672 }], [{ v: 665 }], [{ c: 672 }, { check: "none", timeout: 3e3, wv: 665 }], [], {}], [1, "cookie-popup-cookieok", 1, "", 22, [673], [{ e: 674 }], [{ v: 665 }], [{ h: 665 }], [], {}], [1, "cookie-pref-form", 2, "", 22, [675], [{ e: 676 }], [{ v: 675 }], [{ if: { e: 677 }, then: [{ c: 677 }], else: [{ c: 678 }, { wv: 679 }, { all: true, optional: true, k: 680 }, { c: 679 }] }], [], {}], [1, "cookie-script", 2, "", 22, [681, 682], [{ e: 683 }], [{ v: 684 }], [{ if: { e: 685 }, then: [{ wait: 100 }, { k: 685 }], else: [{ if: { e: 686 }, then: [{ k: 686 }, { timeout: 3e3, optional: true, wv: 687 }, { if: { e: 688 }, then: [{ c: 685 }], else: [{ if: { v: 689 }, then: [{ if: { e: 690 }, then: [{ all: true, k: 690 }] }, { retry: 6, retryInterval: 1e3, c: 689 }], else: [{ k: 691 }] }] }], else: [{ if: { e: 689 }, then: [{ all: true, optional: true, k: 690 }, { k: 689 }], else: [{ k: 691 }] }] }] }], [], {}], [1, "cookie-use-notice", 2, "", 22, [692], [{ e: 693 }], [{ v: 693 }], [{ c: 693 }, { check: "none", timeout: 2e3, wv: 692 }], [{ cc: 694 }], {}], [1, "cookieacceptbar", 1, "", 22, [695], [{ e: 695 }], [{ v: 695 }], [{ h: 695 }], [], {}], [1, "cookiebot.be", 2, "", 22, [696], [{ e: 697 }], [{ v: 696 }], [{ c: 697 }], [{ cc: 698 }], {}], [1, "cookiecode", 2, "", 22, [699], [{ any: [{ e: 700 }, { e: 699 }] }], [{ visible: ["cookiecode-banner", ".cc_root > div"] }], [{ waitForThenClick: ["cookiecode-banner", ".cc_button_rejectall"] }, { waitForVisible: ["cookiecode-banner", ".cc_root > div"], check: "none", timeout: 2e3 }], [{ visible: ["cookiecode-banner", ".cc_root > div"], check: "none" }], {}], [1, "cookieconsent2", 2, "", 22, [701], [{ e: 701 }], [{ e: 702 }, { e: 703 }], [{ if: { e: 704 }, then: [{ c: 704 }], else: [{ c: 705 }, { wait: 500 }, { all: true, optional: true, k: 706 }, { c: 707 }] }], [{ cc: 708 }], {}], [1, "cookieconsent2-legacy", 2, "", 22, [709], [{ e: 710 }], [{ e: 711 }], [{ c: 712 }, { timeout: 1e3, optional: true, w: 713 }, { if: { e: 713 }, then: [{ all: true, optional: true, k: 714 }, { c: 715 }] }], [{ any: [{ cc: 708 }, { cc: 716 }] }], {}], [1, "cookieconsent3", 2, "", 22, [717], [{ e: 717 }], [{ v: 718 }], [{ if: { e: 719 }, then: [{ c: 719 }], else: [{ c: 720 }, { c: 721 }] }], [{ cc: 708 }], {}], [1, "cookiecuttr", 0, "", 10, [722], [{ e: 723 }], [{ check: "any", v: 724 }], [{ if: { e: 725 }, then: [{ k: 725 }], else: [{ h: 722 }] }], [], {}], [1, "cookiefirst.com", 2, "", 22, [726], [{ e: 727 }], [{ check: "any", v: 728 }], [{ if: { e: 729 }, then: [{ k: 729 }], else: [{ if: { e: 730 }, then: [{ timeout: 5e3, wv: 730 }, { k: 730 }, { wait: 1500 }], else: [{ if: { e: 731 }, then: [{ k: 731 }, { timeout: 1e3, wv: 732 }, { eval: "EVAL_COOKIEFIRST_1" }, { wait: 1e3 }, { k: 733 }], else: [{ k: 729 }] }] }] }], [{ eval: "EVAL_COOKIEFIRST_0" }], {}], [1, "cookiehub", 2, "", 22, [734], [{ e: 735 }], [{ v: 735 }], [{ if: { e: 736 }, then: [{ k: 736 }, { wv: 737 }, { if: { e: 738 }, then: [{ k: 738 }], else: [{ all: true, optional: true, k: 739 }, { k: 740 }] }], else: [{ h: 734 }] }], [{ cc: 741 }], {}], [1, "cookieinfo", 1, "", 22, [742], [{ e: 743 }], [{ v: 742 }], [{ c: 744 }], [], {}], [1, "cookieinfo-zamknij", 2, "", 22, [745], [{ e: 746 }], [{ v: 746 }], [{ c: 746 }], [{ cc: 747 }], {}], [1, "cookiejs-banner", 2, "", 22, [748], [{ e: 749 }], [{ v: 750 }], [{ c: 751 }], [{ cc: 752 }], {}], [1, "cookiejs-modal", 2, "", 22, [748], [{ e: 753 }], [{ v: 754 }], [{ c: 755 }], [{ cc: 756 }], {}], [1, "cookielegit", 2, "", 22, [757], [{ e: 757 }], [{ v: 758 }], [{ if: { e: 759 }, then: [{ c: 759 }], else: [{ c: 760 }, { wv: 761 }, { optional: true, k: 762 }, { optional: true, k: 763 }, { k: 761 }] }], [{ cc: 764 }, { cc: 765 }], {}], [1, "cookieman", 2, "", 10, [766], [{ e: 767 }], [{ check: "any", v: 767 }], [{ if: { e: 768 }, then: [{ retry: 3, retryInterval: 500, c: 768 }], else: [{ c: 769 }, { timeout: 2e3, optional: true, wv: 770 }, { all: true, optional: true, k: 771 }, { retry: 3, retryInterval: 500, c: 772 }] }], [{ cc: 773 }, { timeout: 2e3, check: "none", wv: 767 }], {}], [1, "cookiepermission-cp-overlay", 0, "", 10, [774], [{ e: 775 }], [{ v: 774 }], [{ c: 775 }, { check: "none", wv: 774 }], [{ cc: 776 }], {}], [1, "cookieprompter", 0, "", 10, [777], [{ e: 777 }], [{ v: 777 }], [{ if: { e: 778 }, then: [{ k: 778 }], else: [{ c: 779 }] }, { check: "none", wv: 777 }], [{ cc: 780 }], {}], [1, "cookiereports", 2, "", 22, [781, 782], [{ e: 783 }], [{ v: 781 }], [{ eval: "EVAL_COOKIEREPORTS_OPT_OUT" }], [{ eval: "EVAL_COOKIEREPORTS_TEST" }], {}], [1, "cookies-and-content-security-policy", 2, "", 22, [784, 785], [{ e: 786 }], [{ v: 787 }], [{ if: { e: 788 }, then: [{ c: 788 }], else: [{ c: 789 }, { c: 790 }] }], [{ cc: 791 }], {}], [1, "cookies-tip-v1", 2, "", 22, [792], [{ e: 793 }], [{ v: 792 }], [{ c: 793 }], [{ timeout: 1e3, check: "none", wv: 792 }], {}], [1, "cookiesbanner-preference", 2, "", 22, [794], [{ e: 795 }], [{ v: 796 }], [{ c: 797 }, { wv: 798 }, { all: true, optional: true, k: 799 }, { c: 800 }], [{ w: 801 }, { cc: 802 }], {}], [1, "cookiesinfo-closecookies", 2, "", 22, [], [{ e: 803 }], [{ v: 803 }], [{ c: 803 }], [{ check: "none", timeout: 2e3, wv: 804 }], {}], [1, "cookiesjsr", 2, "", 22, [805], [{ e: 806 }], [{ v: 805 }], [{ if: { v: 807 }, then: [{ k: 807 }], else: [{ c: 808 }, { wv: 809 }, { if: { v: 810 }, then: [{ k: 810 }], else: [{ all: true, optional: true, k: 811 }, { c: 812 }] }] }, { timeout: 2e3, check: "none", wv: 805 }], [], {}], [1, "cookiesmodal-pl", 2, "", 22, [], [{ e: 813 }, { e: 814 }], [{ v: 815 }], [{ if: { e: 816 }, then: [{ k: 814 }] }, { retry: 3, retryInterval: 600, c: 817 }], [{ cc: 818 }, { negated: true, cc: 819 }], {}], [1, "cookiesplus", 2, "", 22, [820, 821], [{ e: 822 }], [{ v: 823 }], [{ if: { check: "any", v: 824 }, then: [{ k: 824 }], else: [{ k: 825 }, { c: 826 }] }, { check: "none", timeout: 5e3, wv: 823 }], [{ cc: 827 }], {}], [1, "cookietractor", 2, "", 22, [828], [{ e: 829 }], [{ v: 829 }], [{ all: true, optional: true, k: 830 }, { c: 831 }], [{ timeout: 1e3, check: "none", wv: 829 }], {}], [1, "cookieyes", 2, "", 22, [832], [{ e: 833 }], [{ v: 833 }], [{ if: { e: 834 }, then: [{ c: 834 }], else: [{ if: { e: 835 }, then: [{ c: 835 }, { w: 836 }, { all: true, optional: true, k: 837 }, { c: 838 }, { optional: true, wv: 839 }, { optional: true, c: 840 }], else: [{ if: { e: 841 }, then: [{ k: 841 }, { w: 842 }, { all: true, optional: true, k: 843 }, { c: 844 }], else: [{ h: 845 }] }] }] }], [{ cc: 846 }], {}], [1, "cookieyes-legacy", 2, "", 22, [847], [{ e: 847 }], [{ v: 847 }], [{ if: { v: 848 }, then: [{ c: 848 }], else: [{ c: 849 }, { wv: 850 }, { all: true, optional: true, k: 851 }, { k: 850 }] }], [{ cc: 852 }, { any: [{ cc: 853 }, { cc: 854 }] }], {}], [1, "cookiez", 2, "", 22, [855], [{ e: 856 }], [{ v: 857 }], [{ c: 857 }], [{ wait: 1e3 }, { cc: 858 }], {}], [1, "coolblue", 0, "", 10, [859], [{ e: 860 }], [{ v: 861 }], [{ c: 862 }], [], {}], [1, "cornerstone-brands", 2, "", 22, [863, 864], [{ any: [{ e: 865 }, { e: 866 }] }], [{ any: [{ v: 863 }, { v: 864 }] }], [{ timeout: 5e3, optional: true, c: 865 }, { timeout: 6e3, optional: true, c: 866 }], [{ check: "none", timeout: 2e3, wv: 867 }], {}], [1, "corona-in-zahlen.de", 2, "", 22, [868], [{ e: 868 }], [{ v: 868 }], [{ k: 869 }, { k: 870 }], [], {}], [1, "costco-travel", 2, "", 22, [], [{ e: 871 }, { e: 872 }], [{ v: 873 }], [{ c: 871 }, { wv: 874 }, { all: true, optional: true, k: 875 }, { all: true, optional: true, k: 876 }, { k: 874 }], [], {}], [1, "craft-cookie-consent", 2, "", 22, [877], [{ e: 878 }], [{ v: 879 }], [{ all: true, optional: true, k: 880 }, { if: { v: 881 }, then: [{ k: 881 }], else: [{ k: 882 }, { wv: 883 }, { if: { v: 884 }, then: [{ k: 884 }], else: [{ k: 881 }] }] }], [{ w: 885 }], {}], [1, "ct-ultimate-gdpr", 2, "", 22, [886], [{ e: 886 }], [{ timeout: 3e4, wv: 886 }], [{ if: { v: 887 }, then: [{ k: 887 }], else: [{ if: { v: 888 }, then: [{ k: 888 }, { c: 889 }, { k: 890 }], else: [{ h: 886 }] }] }], [{ wait: 500 }, { cc: 891 }], {}], [1, "curseforge", 1, "", 22, [892], [{ e: 893 }], [{ v: 893 }], [{ h: 892 }], [], {}], [1, "cwarn", 1, "", 22, [894], [{ e: 895 }], [{ v: 894 }], [{ h: 894 }], [], {}], [1, "cz-eshop-cookies-consent", 2, "", 22, [896], [{ e: 897 }], [{ v: 898 }], [{ retry: 10, retryInterval: 500, c: 897 }], [{ cc: 899 }, { negated: true, cc: 900 }], {}], [1, "dack365", 2, "", 22, [901], [{ e: 902 }], [{ v: 903 }], [{ c: 903 }, { c: 904 }], [{ check: "none", timeout: 5e3, wv: 905 }], {}], [1, "daikin", 2, "", 22, [906], [{ e: 907 }], [{ v: 906 }], [{ c: 907 }], [{ cc: 908 }], {}], [1, "dailymotion-us", 1, "", 22, [909], [{ e: 910 }], [{ v: 910 }], [{ h: 910 }], [], {}], [1, "dan-com", 2, "", 10, [], [{ e: 911 }], [{ v: 911 }], [{ c: 912 }], [], {}], [1, "dandomain", 0, "", 22, [913], [{ e: 914 }], [{ v: 913 }], [{ wait: 500 }, { all: true, optional: true, k: 915 }, { c: 914 }], [{ cc: 916 }], {}], [1, "danskebank", 0, "", 10, [917], [{ e: 918 }], [{ v: 919 }], [{ c: 920 }, { check: "none", wv: 919 }], [{ cc: 921 }], {}], [1, "datagrail", 2, "", 22, [922], [{ e: 922 }, { exists: [".dg-consent-banner", ".dg-app"] }], [{ v: 922 }], [{ if: { exists: [".dg-consent-banner", "button.dg-button.reject_all, button.dg-button.accept_some"] }, then: [{ waitForThenClick: [".dg-consent-banner", "button.dg-button.reject_all, button.dg-button.accept_some"] }], else: [{ if: { exists: [".dg-consent-banner", ".dg-browser-signal-notice"] }, then: [{ if: { exists: [".dg-consent-banner", "button.dg-button.open_layer"] }, then: [{ waitForThenClick: [".dg-consent-banner", "button.dg-button.open_layer"] }, { waitForThenClick: [".dg-consent-banner", "button.dg-button.reject_all, button.dg-button.accept_some, button.dg-button.essential_only, button.dg-button.custom"] }], else: [{ waitForThenClick: [".dg-consent-banner", "button.dg-header-close"] }] }], else: [{ waitForThenClick: [".dg-consent-banner", "button.dg-button.open_layer"] }, { waitForThenClick: [".dg-consent-banner", "button.dg-button.reject_all, button.dg-button.accept_some, button.dg-button.essential_only"] }] }] }], [{ any: [{ cc: 923 }, { exists: [".dg-consent-banner", ".dg-browser-signal-notice"] }] }], {}], [1, "datasign", 0, "", 22, [], [{ e: 924 }], [{ visible: ["#datasign_cmp__host", "#datasign_cmp__cmp_summary"] }], [{ if: { exists: ["#datasign_cmp__host", "#datasign_cmp__cmp_content input[type=checkbox]:not([disabled])"] }, then: [{ waitForThenClick: ["#datasign_cmp__host", "#datasign_cmp__config-button, #datasign_cmp__cmp_summary [aria-controls=datasign_cmp__cmp_content]"] }, { waitForVisible: ["#datasign_cmp__host", "#datasign_cmp__cmp_content_apply-button"] }, { click: ["#datasign_cmp__host", "#datasign_cmp__cmp_content input[type=checkbox]:checked:not([disabled])"], all: true, optional: true }, { click: ["#datasign_cmp__host", "#datasign_cmp__cmp_content_apply-button"] }], else: [{ click: ["#datasign_cmp__host", "#datasign_cmp__cmp_summary .datasign__dialogCloseButton--index-module"] }] }], [{ waitForVisible: ["#datasign_cmp__host", "#datasign_cmp__cmp_summary"], check: "none", timeout: 3e3 }], {}], [1, "de-volksbank", 2, "", 22, [], [{ e: 925 }], [{ e: 926 }], [{ c: 927 }], [{ cc: 928 }], {}], [1, "dfactory-cookie-notice", 2, "", 22, [929], [{ e: 930 }], [{ v: 931 }], [{ if: { e: 932 }, then: [{ c: 932 }], else: [{ c: 933 }] }, { check: "none", timeout: 1e3, optional: true, wv: 934 }, { if: { v: 934 }, then: [{ h: 934 }] }], [{ cc: 935 }], {}], [1, "didomi", 2, "", 22, [936], [{ e: 937 }], [{ check: "any", v: 938 }], [{ if: { e: 939 }, then: [{ c: 939 }], else: [{ if: { e: 940 }, then: [{ c: 941 }, { c: 942 }], else: [{ eval: "EVAL_DIDOMI_OPT_OUT" }] }] }], [{ eval: "EVAL_DIDOMI_TEST" }], {}], [1, "diffuse-cookie-notice", 2, "", 22, [943], [{ e: 944 }], [{ v: 944 }], [{ if: { e: 945 }, then: [{ k: 945 }], else: [{ k: 946 }, { wv: 947 }, { all: true, optional: true, k: 948 }, { c: 949 }] }], [{ negated: true, cc: 950 }], {}], [1, "dintel", 2, "", 22, [951], [{ exists: ["dnt-cookies", ".dnt-cookies__footer"] }], [{ visible: ["dnt-cookies", ".dnt-cookies--is-open"] }], [{ waitForThenClick: ["dnt-cookies", ".dnt-cookies__footer dnt-button[type^=secondary]"] }, { waitForVisible: ["dnt-cookies", ".dnt-cookies--is-open"], check: "none", timeout: 2e3, optional: true }], [{ cc: 952 }], {}], [1, "django-gdpr-cookie-consent", 2, "", 22, [953, 954], [{ e: 955 }], [{ v: 955 }], [{ if: { e: 956 }, then: [{ c: 956 }], else: [{ c: 957 }, { c: 958 }, { k: 959 }] }, { check: "none", timeout: 1e4, wv: 960 }], [{ cc: 961 }], {}], [1, "dlibra", 2, "", 22, [], [{ e: 962 }, { e: 963 }], [{ v: 963 }], [{ c: 963 }], [{ cc: 964 }], {}], [1, "dmgmedia", 2, "", 22, [965], [{ e: 966 }], [{ v: 966 }], [{ c: 967 }, { wv: 968 }, { all: true, c: 969 }, { waitForThenClick: ['[data-project="mol-fe-cmp"] [class*=footer]', "xpath///button[contains(., 'Save & Exit')]"] }], [], {}], [1, "dmgmedia-us", 2, "", 22, [970], [{ e: 971 }], [{ wv: 971 }], [{ c: 972 }, { wv: 973 }, { c: 974 }, { c: 975 }], [], {}], [1, "dnb", 2, "", 22, [976], [{ e: 977 }], [{ v: 977 }], [{ c: 977 }], [{ cc: 978 }], {}], [1, "doordash-storefront", 2, "", 22, [979], [{ e: 979 }], [{ v: 979 }], [{ retry: 15, retryInterval: 300, c: 980 }], [{ cc: 981 }], {}], [1, "dpgmedia-nl", 2, "", 22, [982], [{ e: 982 }], [{ visible: ["#pg-shadow-host-dom", "#pg-modal"] }], [{ if: { visible: ["#pg-shadow-host-dom", "#pg-reject-btn"] }, then: [{ click: ["#pg-shadow-host-dom", "#pg-reject-btn"] }], else: [{ waitForThenClick: ["#pg-shadow-host-dom", "#pg-configure-btn"] }, { waitForThenClick: ["#pg-shadow-host-dom", "#pg-reject-btn"] }] }], [], {}], [1, "dreamlab-cmp", 2, "", 22, [983], [{ e: 984 }], [{ v: 983 }], [{ c: 985 }, { c: 986 }], [{ cc: 987 }], {}], [1, "drouot-rgpd", 2, "", 22, [988, 989], [{ e: 990 }], [{ v: 988 }], [{ c: 991 }], [{ cc: 992 }], {}], [1, "Drupal", 2, "", 22, [], [{ e: 993 }], [{ v: 993 }], [{ k: 994 }], [], {}], [1, "duda-cookie-notification", 0, "", 10, [995], [{ e: 996 }], [{ v: 997 }], [{ retry: 3, retryInterval: 500, c: 998 }], [{ check: "none", v: 999 }], { intermediate: false }], [1, "dunelm.com", 2, "", 22, [1e3], [{ e: 1001 }], [{ v: 1001 }], [{ k: 1002 }, { k: 1003 }], [{ cc: 1004 }, { cc: 1005 }], {}], [1, "e-file", 2, "", 22, [1006], [{ e: 1007 }], [{ v: 1008 }], [{ c: 1008 }, { wv: 1007 }, { all: true, optional: true, k: 1009 }, { k: 1007 }], [{ cc: 1010 }, { cc: 1011 }], {}], [1, "e-grip", 2, "", 22, [], [{ e: 1012 }], [{ v: 1013 }], [{ if: { e: 1014 }, then: [{ c: 1014 }], else: [{ c: 1015 }, { timeout: 2e3, w: 1016 }, { if: { e: 1017 }, then: [{ k: 1018 }] }, { k: 1019 }] }], [{ cc: 961 }, { negated: true, cc: 1020 }], {}], [1, "eagerly-tools-cookie", 2, "", 22, [1021, 1022], [{ e: 1021 }], [{ v: 1023 }], [{ all: true, optional: true, k: 1024 }, { c: 1025 }], [{ check: "none", wv: 1023 }], {}], [1, "easy-cookie-info", 1, "", 22, [1026], [{ e: 1026 }], [{ v: 1026 }], [{ h: 1026 }], [], {}], [1, "ecbeing", 1, "", 22, [1027], [{ e: 1028 }], [{ v: 1029 }], [{ h: 1029 }], [], {}], [1, "eccmm", 2, "", 22, [], [{ exists: ["#eccm-shadow-host", ".eccmm"] }], [{ visible: ["#eccm-shadow-host", ".eccmm-dialog.eccmm-visible"] }], [{ waitForThenClick: ["#eccm-shadow-host", ".eccmm-dialog .eccmm-deny-all-btn--footer"] }], [{ waitForVisible: ["#eccm-shadow-host", ".eccmm-dialog"], check: "none", timeout: 2e3 }], {}], [1, "econda-pp2", 0, "", 22, [1030, 1031], [{ e: 1032 }], [{ check: "any", v: 1033 }], [{ c: 1033 }], [{ cc: 1034 }], {}], [1, "educamadrid", 2, "", 22, [1035], [{ e: 1036 }], [{ v: 1037 }], [{ c: 1038 }, { c: 1039 }], [{ timeout: 3e3, w: 1040 }, { cc: 1041 }], {}], [1, "ee-consent-notice", 2, "", 22, [1042], [{ e: 1043 }], [{ v: 1044 }], [{ if: { e: 1045 }, then: [{ c: 1045 }], else: [{ eval: "EVAL_EE_CONSENT_DECLINE_0" }, { c: 1046 }] }], [], {}], [1, "elsevier-pure", 2, "", 22, [1047], [{ e: 1048 }], [{ v: 1048 }], [{ c: 1049 }], [{ timeout: 1e3, check: "none", wv: 1048 }], {}], [1, "Ensighten", 2, "", 22, [1050, 1051], [{ any: [{ v: 1052 }, { v: 1053 }] }], [{ any: [{ v: 1052 }, { v: 1053 }] }], [{ wait: 500 }, { if: { v: 1052 }, then: [{ if: { check: "any", v: 1054 }, then: [{ c: 1054 }], else: [{ if: { check: "any", v: 1055 }, then: [{ c: 1055 }, { timeout: 1e3, w: 1053 }, { all: true, optional: true, k: 1056 }, { c: 1057 }], else: [{ c: 1058 }] }] }], else: [{ all: true, optional: true, k: 1056 }, { c: 1057 }] }], [], {}], [1, "enzuzo", 2, "", 22, [1059], [{ e: 1059 }], [{ v: 1059 }], [{ if: { v: 1060 }, then: [{ k: 1060 }], else: [{ k: 1061 }, { c: 1062 }] }], [{ wait: 500 }, { cc: 1063 }], {}], [1, "epaas", 2, "", 22, [1064], [{ e: 1064 }], [{ visible: ["epaas-consent-drawer-shell", "section.consentDrawer"] }], [{ if: { exists: ["epaas-consent-drawer-shell", "button.reject-button"] }, then: [{ waitForThenClick: ["epaas-consent-drawer-shell", "button.reject-button"] }], else: [{ waitForThenClick: ["epaas-consent-drawer-shell", "button.personalize-button"] }, { waitFor: ["epaas-consent-drawer-shell", "epaas-policy-page-shell"] }, { waitFor: ["epaas-consent-drawer-shell", "epaas-policy-page-shell", "label[class*='category-header__switch']"], optional: true }, { click: ["epaas-consent-drawer-shell", "epaas-policy-page-shell", "label.category-header__switch--on input"], all: true, optional: true }, { waitFor: ["epaas-consent-drawer-shell", "section.consentDrawer.layered button.accept-button"] }, { if: { exists: ["epaas-consent-drawer-shell", "button.save-button"] }, then: [{ click: ["epaas-consent-drawer-shell", "button.save-button"] }], else: [{ waitFor: ["epaas-consent-drawer-shell", "epaas-policy-page-shell", "label.category-header__switch--off"] }, { click: ["epaas-consent-drawer-shell", "button.accept-button"] }] }] }], [{ cc: 1065 }], {}], [1, "erdee-media", 2, "", 22, [1066], [{ e: 1066 }], [{ v: 1067 }], [{ c: 1068 }], [], {}], [1, "eset", 0, "", 22, [1069], [{ e: 1070 }], [{ v: 1070 }], [{ if: { v: 1071 }, then: [{ k: 1071 }], else: [{ k: 1072 }, { wv: 1073 }, { all: true, optional: true, k: 1074 }, { k: 1073 }] }], [{ wait: 2e3 }, { cc: 1075 }], {}], [1, "etracker", 2, "", 22, [1076, 1077], [{ e: 1078 }], [{ v: 1078 }], [{ c: 1078 }], [{ timeout: 3e3, check: "none", wv: 1076 }, { cc: 1079 }], {}], [1, "etsy", 2, "", 22, [1080, 1081], [{ e: 1080 }], [{ v: 1080 }], [{ k: 1082 }, { timeout: 3e3, wv: 1083 }, { wait: 1e3 }, { eval: "EVAL_ETSY_0" }, { eval: "EVAL_ETSY_1" }], [], {}], [1, "EU Cookie Law", 1, "", 22, [1084], [{ e: 1085 }], [{ wait: 500 }, { v: 1085 }], [{ h: 1085 }], [{ negated: true, cc: 1086 }], {}], [1, "eu-cookie-compliance-banner", 2, "", 22, [], [{ e: 1087 }], [{ e: 1087 }], [{ k: 1088 }], [{ negated: true, cc: 1089 }], {}], [1, "eu-cookie-law-popup", 2, "", 22, [1090], [{ e: 1091 }], [{ check: "any", v: 1091 }], [{ c: 1091 }], [{ check: "none", timeout: 2e3, wv: 1091 }], {}], [1, "EZoic", 2, "", 22, [1092], [{ e: 1092 }], [{ v: 1092 }], [{ wait: 500 }, { k: 1093 }, { w: 1094 }, { all: true, optional: true, k: 1095 }, { c: 1096 }], [{ cc: 1097 }], {}], [1, "fabrique-cookie-message", 2, "", 22, [1098], [{ e: 1099 }], [{ v: 1099 }], [{ c: 1099 }], [{ any: [{ cc: 1100 }, { cc: 1101 }] }], {}], [1, "fanatics", 2, "", 22, [1102], [{ e: 1102 }], [{ v: 1102 }], [{ c: 1103 }], [{ cc: 1104 }], {}], [1, "fastcmp", 0, "", 22, [1105], [{ e: 1106 }], [{ v: 1106 }], [{ waitForThenClick: ["iframe#fast-cmp-iframe", ".fast-cmp-home-refuse button"] }], [{ cc: 1107 }], {}], [1, "fastweb-cookie-center", 2, "", 22, [1108], [{ e: 1109 }], [{ v: 1110 }], [{ waitForThenClick: ["iframe.ck_if", "#Main > .close img"] }, { check: "none", timeout: 5e3, wv: 1108 }], [{ check: "none", v: 1111 }], {}], [1, "fedex", 0, "", 22, [1112], [{ e: 1112 }], [{ v: 1113 }], [{ c: 1114 }], [{ cc: 1115 }], {}], [1, "fera-cookies", 2, "", 22, [1116], [{ e: 1117 }], [{ v: 1118 }], [{ if: { v: 1119 }, then: [{ k: 1119 }], else: [{ c: 1120 }] }, { timeout: 2e3, optional: true, c: 1121 }], [{ cc: 1122 }, { negated: true, cc: 1123 }], {}], [1, "fever", 2, "", 22, [1124], [{ e: 1125 }], [{ v: 1124 }], [{ c: 1125 }, { if: { e: 1126 }, then: [{ k: 1127 }] }, { c: 1128 }], [{ cc: 1129 }], {}], [1, "fever-cookie-advice", 2, "", 22, [1130], [{ e: 1130 }], [{ v: 1130 }], [{ c: 1131 }, { wv: 1132 }, { c: 1133 }], [{ wait: 500 }, { cc: 1134 }], {}], [1, "fides", 2, "", 22, [1135], [{ e: 1136 }], [{ v: 1136 }, { check: "none", v: 1137 }, { eval: "EVAL_FIDES_DETECT_POPUP" }], [{ w: 1138 }, { if: { v: 1139 }, then: [{ retry: 5, retryInterval: 500, c: 1140 }], else: [{ k: 1141 }, { retry: 5, retryInterval: 500, c: 1142 }] }], [], {}], [1, "finna", 2, "", 22, [1143], [{ e: 1144 }], [{ v: 1144 }], [{ retry: 10, retryInterval: 500, c: 1144 }], [], {}], [1, "finsweet", 0, "", 22, [1145], [{ any: [{ e: 1145 }, { exists: ["[fs-consent-element=root]", "[fs-consent-element=banner]"] }] }], [{ any: [{ v: 1146 }, { visible: ["[fs-consent-element=root]", "[fs-consent-element=banner] [fs-consent-element=allow]"] }] }], [{ wait: 500 }, { if: { e: 1147 }, then: [{ k: 1147 }], else: [{ if: { exists: ["[fs-consent-element=root]", "[fs-consent-element=banner] [fs-consent-element=deny]"] }, then: [{ click: ["[fs-consent-element=root]", "[fs-consent-element=banner] [fs-consent-element=deny]"] }], else: [{ h: 1148 }] }] }], [], {}], [1, "first-visit-notice", 1, "", 22, [1149], [{ e: 1150 }], [{ v: 1149 }], [{ h: 1149 }], [], {}], [1, "fixpart", 2, "", 22, [1151], [{ e: 1152 }], [{ v: 1151 }], [{ c: 1153 }], [{ cc: 1154 }], {}], [1, "flatsome-cookies", 1, "", 22, [1155], [{ e: 1156 }], [{ v: 1157 }], [{ h: 1155 }], [], {}], [1, "fnsp-cookieconsent", 2, "", 22, [1158], [{ e: 1159 }], [{ v: 1160 }], [{ c: 1161 }], [{ cc: 1162 }], {}], [1, "ford-dse", 2, "", 22, [1163], [{ e: 1164 }], [{ v: 1165 }], [{ c: 1164 }, { w: 1166 }], [{ cc: 1167 }], {}], [1, "fullertonhotels.com", 0, "", 10, [], [{ exists: ["#ifrmCookieBanner", "#sp-decline"] }], [{ visible: ["#ifrmCookieBanner", "#sp-decline"] }], [{ wait: 500 }, { waitForThenClick: ["#ifrmCookieBanner", "#sp-decline"] }], [], {}], [1, "funding-choices", 2, "", 22, [1168], [{ e: 1169 }], [{ e: 1170 }], [{ if: { e: 1171 }, then: [{ k: 1171 }], else: [{ e: 1172 }] }, { all: true, optional: true, k: 1173 }, { optional: true, k: 1174 }], [], {}], [1, "gallup", 2, "", 22, [1175], [{ e: 1176 }], [{ v: 1176 }], [{ c: 1176 }, { wait: 500 }, { c: 1177 }], [{ cc: 1178 }], {}], [1, "gameforge", 2, "", 22, [1179], [{ e: 1180 }, { e: 1181 }], [{ v: 1182 }], [{ c: 1181 }, { wv: 1183 }, { c: 1184 }], [{ cc: 1185 }, { cc: 1186 }], {}], [1, "gdpr-jp-notice", 1, "", 22, [1187], [{ e: 1188 }], [{ v: 1187 }], [{ h: 1187 }], [], {}], [1, "gdpr-legal-cookie", 2, "", 22, [1189], [{ any: [{ eval: "EVAL_GDPR_LEGAL_COOKIE_DETECT_CMP" }, { e: 1190 }] }], [{ check: "any", v: 1191 }], [{ c: 1192 }], [{ eval: "EVAL_GDPR_LEGAL_COOKIE_TEST" }], {}], [1, "gdpsystem", 2, "", 22, [1193], [{ e: 1194 }], [{ v: 1195 }], [{ if: { e: 1196 }, then: [{ c: 1196 }], else: [{ c: 1197 }, { wv: 1198 }, { all: true, optional: true, k: 1199 }, { c: 1200 }] }], [{ check: "none", wv: 1195 }], {}], [1, "gear4music", 2, "", 10, [1201], [{ e: 1202 }], [{ v: 1201 }], [{ c: 1203 }, { check: "none", timeout: 2e3, wv: 1201 }], [{ wait: 1e3 }, { cc: 1204 }], {}], [1, "gemini.google.com", 2, "", 22, [1205], [{ e: 1206 }], [{ v: 1205 }], [{ c: 1207 }], [], {}], [1, "generalitat-catalunya", 2, "", 22, [1208], [{ e: 1209 }], [{ v: 1209 }], [{ c: 1210 }, { c: 1211 }], [{ cc: 1212 }], {}], [2, "getproven", 1, "", 22, [1213], [{ e: 1214 }], [{ v: 1214 }], [{ h: 1213 }, { removeClass: "cdk-global-scrollblock", selector: "html", optional: true }], [], {}], [1, "giustizia-it", 2, "", 22, [1215], [{ e: 1216 }], [{ v: 1217 }], [{ all: true, optional: true, k: 1218 }, { c: 1219 }], [{ cc: 1220 }], {}], [1, "gobasic", 2, "", 22, [1221], [{ e: 1222 }], [{ v: 1222 }], [{ c: 1222 }], [{ cc: 1223 }], {}], [1, "godaddy-privacy-widget", 2, "", 22, [1224], [{ exists: ["#gtm_privacy", "#privacy_widget"] }], [{ visible: ["#gtm_privacy", "#privacy_widget"] }], [{ waitForThenClick: ["#gtm_privacy", "#pw_decline"] }], [{ cc: 1225 }], {}], [1, "godaddy-website-builder", 2, "", 22, [1226], [{ e: 1227 }], [{ v: 1228 }], [{ c: 1227 }], [], {}], [1, "godaddy-website-builder-cosmetic", 1, "", 22, [1226], [{ e: 1226 }, { negated: true, e: 1227 }], [{ v: 1228 }], [{ h: 1226 }], [], {}], [1, "google-consent-standalone", 2, "", 22, [], [{ e: 1229 }, { e: 1230 }], [{ v: 1229 }], [{ c: 1231 }], [], {}], [1, "google-cookiebar", 0, "", 22, [1232], [{ e: 1232 }], [{ v: 1232 }], [{ if: { e: 1233 }, then: [{ k: 1233 }], else: [{ h: 1232 }] }], [], {}], [1, "google.com", 2, "", 22, [1234], [{ e: 1234 }, { e: 1235 }], [{ v: 1236 }], [{ c: 1236 }], [{ cc: 1237 }], {}], [1, "gov.uk", 2, "", 22, [], [{ e: 1238 }], [{ e: 1239 }], [{ wait: 300 }, { if: { visible: [".govuk-cookie-banner__message", "xpath///button[contains(., 'Reject')] | //a[contains(., 'Reject')] | //input[contains(@value, 'Reject')] | //button[contains(., 'do not use analytics cookies')]"] }, then: [{ click: [".govuk-cookie-banner__message", "xpath///button[contains(., 'Reject')] | //a[contains(., 'Reject')] | //input[contains(@value, 'Reject')] | //button[contains(., 'do not use analytics cookies')]"] }] }, { waitForVisible: [".govuk-cookie-banner__message", "xpath///button[contains(., 'Hide')] | //a[contains(., 'Hide')] | //input[contains(@value, 'Hide')]"] }, { click: [".govuk-cookie-banner__message", "xpath///button[contains(., 'Hide')] | //a[contains(., 'Hide')] | //input[contains(@value, 'Hide')]"] }], [], {}], [1, "gravito", 2, "", 22, [1240], [{ e: 1241 }], [{ v: 1241 }], [{ if: { v: 1242 }, then: [{ k: 1242 }], else: [{ c: 1243 }, { c: 1244 }] }, { check: "none", timeout: 2e3, optional: true, wv: 1241 }], [{ cc: 1245 }], {}], [1, "grrr-cookie-consent", 2, "", 22, [], [{ exists: ["wookie-consent, cookie-consent", "aside.js-cookie-bar form button"] }], [{ visible: ["wookie-consent, cookie-consent", "aside.js-cookie-bar[aria-hidden=false]"] }], [{ click: ["wookie-consent, cookie-consent", "aside.js-cookie-bar input[type=checkbox]:checked:not([disabled])"], all: true, optional: true }, { waitForThenClick: ["wookie-consent, cookie-consent", "aside.js-cookie-bar form button"], retry: 2 }], [{ exists: ["wookie-consent, cookie-consent", "aside.js-cookie-bar[aria-hidden=true]"] }], {}], [1, "grupa-pracuj", 0, "", 22, [], [{ e: 1246 }], [{ check: "any", v: 1246 }], [{ if: { e: 1247 }, then: [{ c: 1248 }, { wv: 1249 }, { all: true, optional: true, k: 1250 }, { c: 1251 }], else: [{ c: 1252 }, { wv: 1253 }, { all: true, optional: true, k: 1254 }, { c: 1255 }] }], [{ any: [{ cc: 1256 }, { cc: 1257 }] }], {}], [1, "grupa-rmf", 2, "", 22, [1258], [{ e: 1259 }], [{ v: 1258 }], [{ wv: 1260 }, { k: 1260 }, { wv: 1261 }, { k: 1261 }, { wv: 1262 }, { if: { e: 1263 }, then: [{ k: 1262 }] }, { wv: 1264 }, { k: 1264 }, { check: "none", wv: 1258 }], [], {}], [1, "gsb-cookie-banner", 2, "", 22, [1265], [{ e: 1266 }], [{ check: "any", v: 1267 }], [{ c: 1267 }], [{ cc: 375 }], {}], [1, "gsb-cookiebanner-selection", 2, "", 22, [1268, 1269], [{ e: 1270 }], [{ v: 1270 }], [{ all: true, optional: true, k: 1271 }, { c: 1270 }], [{ cc: 1272 }], {}], [1, "guesty", 0, "", 22, [1273], [{ e: 1274 }], [{ v: 1273 }], [{ c: 1275 }], [{ cc: 1276 }], {}], [1, "gwo", 2, "", 22, [1277], [{ e: 1277 }], [{ v: 1278 }], [{ if: { e: 1279 }, then: [], else: [{ c: 1280 }] }, { w: 1281 }, { all: true, optional: true, k: 1282 }, { optional: true, k: 1283 }, { k: 1280 }], [{ cc: 1284 }], {}], [1, "hacon-next-gen", 2, "", 22, [], [{ exists: ["next-gen-map-based-route", "next-gen-cookie-dialog"] }], [{ visible: ["next-gen-map-based-route", "next-gen-cookie-dialog next-gen-page.cookie-dialog"] }], [{ if: { exists: ["next-gen-map-based-route", "next-gen-cookie-dialog input[type=checkbox]:checked"] }, then: [{ click: ["next-gen-map-based-route", "next-gen-cookie-dialog input[type=checkbox]:checked"], all: true }] }, { waitForThenClick: ["next-gen-map-based-route", "next-gen-cookie-dialog next-gen-page-footer button[variant=secondary]"] }], [{ cc: 1285 }], {}], [1, "hallmark", 2, "", 22, [1286], [{ e: 1287 }], [{ v: 1286 }], [{ c: 1287 }, { wv: 1288 }, { k: 1289 }, { k: 1288 }, { check: "none", timeout: 3e3, optional: true, wv: 1286 }], [{ cc: 1290 }], {}], [1, "handelsbanken", 2, "", 22, [1291], [{ e: 1292 }], [{ v: 1292 }], [{ c: 1293 }], [{ cc: 1294 }], {}], [1, "healthline-media", 2, "", 22, [1295], [{ e: 1296 }], [{ e: 1296 }], [{ if: { e: 1297 }, then: [{ k: 1297 }], else: [{ wv: 1298 }, { k: 1299 }] }], [], {}], [1, "help-ch", 2, "", 22, [1300], [{ e: 1301 }], [{ v: 1301 }], [{ c: 1301 }], [], {}], [1, "hema", 2, "", 22, [376], [{ v: 1302 }], [{ v: 1302 }], [{ c: 1303 }], [{ cc: 1304 }], {}], [1, "hkdir", 2, "", 22, [1305], [{ e: 1306 }, { e: 1307 }], [{ v: 1305 }], [{ c: 1308 }], [{ cc: 1309 }], {}], [1, "hl.co.uk", 2, "", 22, [1310, 1311], [{ e: 1311 }], [{ e: 1311 }], [{ k: 1312 }, { h: 1313 }, { w: 1314 }, { optional: true, k: 1315 }, { w: 1316 }, { optional: true, k: 1317 }, { k: 1318 }], [], {}], [1, "hmgroup-turbocookie", 2, "", 22, [1319], [{ e: 1320 }], [{ v: 1319 }], [{ if: { e: 1321 }, then: [{ c: 1321 }], else: [{ c: 1322 }, { c: 1323 }] }], [{ wait: 500 }, { cc: 1324 }], {}], [1, "hmrc-tracking-consent", 2, "", 22, [1325], [{ e: 1326 }], [{ v: 1326 }], [{ c: 1326 }, { timeout: 3e3, optional: true, c: 1327 }], [{ cc: 1328 }], {}], [1, "holidaymedia", 2, "", 22, [1329], [{ e: 1330 }], [{ v: 1329 }], [{ timeout: 2e3, c: 1331 }], [], {}], [1, "holidu", 2, "", 22, [1332], [{ e: 1333 }], [{ v: 1332 }], [{ c: 1333 }], [{ cc: 1334 }, { cc: 1335 }], {}], [1, "hometogo", 2, "", 22, [1336], [{ e: 1337 }], [{ v: 1336 }], [{ if: { e: 1338 }, then: [{ c: 1338 }], else: [{ c: 1339 }, { c: 1340 }] }], [{ cc: 961 }], {}], [1, "hu-manity", 2, "", 22, [1341], [{ e: 1342 }], [{ v: 1342 }], [{ c: 1343 }], [], {}], [1, "hubspot", 2, "", 22, [], [{ e: 1344 }], [{ v: 1344 }], [{ if: { e: 1345 }, then: [{ k: 1345 }], else: [{ v: 1346 }, { k: 1346 }] }], [], {}], [1, "hurra-consent", 2, "", 22, [1347], [{ e: 1348 }], [{ v: 1347 }], [{ if: { v: 1349 }, then: [{ k: 1349 }], else: [{ c: 1350 }] }], [{ timeout: 2e3, check: "none", wv: 1347 }], {}], [1, "ibkr", 2, "", 22, [1351], [{ e: 1352 }], [{ v: 1351 }], [{ if: { v: 1353 }, then: [{ k: 1353 }], else: [{ k: 1352 }, { c: 1354 }] }], [{ cc: 1355 }], {}], [1, "idxrcookies", 2, "", 22, [1356], [{ e: 1357 }], [{ v: 1357 }], [{ c: 1357 }], [], {}], [1, "ihavecookies", 2, "", 22, [1358], [{ e: 1359 }], [{ v: 1358 }], [{ c: 1359 }, { wv: 1360 }, { all: true, optional: true, k: 1361 }, { if: { v: 1362 }, then: [{ k: 1362 }], else: [{ k: 1363 }] }, { check: "none", timeout: 2e3, optional: true, wv: 1358 }, { if: { e: 1364 }, then: [{ timeout: 5e3, optional: true, c: 1365 }] }], [{ cc: 1366 }], {}], [1, "ilovepdf", 2, "", 22, [], [{ e: 1367 }], [{ v: 1367 }], [{ c: 1367 }, { c: 1368 }], [{ cc: 1369 }], {}], [1, "imarina", 1, "", 22, [1370], [{ e: 1371 }], [{ v: 1370 }], [{ h: 1370 }], [], {}], [1, "impress-co-jp", 2, "", 22, [1372], [{ e: 1373 }], [{ v: 1373 }], [{ c: 1373 }], [{ cc: 1374 }], {}], [1, "imyfone", 1, "", 22, [1375], [{ e: 1376 }], [{ v: 1375 }], [{ h: 1375 }], [], {}], [1, "in2cookiemodal", 2, "", 22, [1377], [{ e: 1377 }], [{ v: 1377 }], [{ if: { v: 1378 }, then: [{ k: 1378 }], else: [{ all: true, optional: true, k: 1379 }, { c: 1380 }] }, { check: "none", timeout: 2e3, optional: true, wv: 1377 }], [{ cc: 1381 }], {}], [1, "incomit", 2, "", 22, [1382], [{ exists: ["cookie-consent-element", ".incom-cookie-module"] }], [{ visible: ["cookie-consent-element", ".incom-cookie-module #main-cookie-view"] }], [{ waitForThenClick: ["cookie-consent-element", "#main-cookie-view nav button.btn:not(#cookie-accept-all)"] }], [{ cc: 1383 }], {}], [1, "indy-systems", 1, "", 22, [1384], [{ e: 1385 }], [{ v: 1384 }], [{ h: 1384 }], [], {}], [1, "infomaniak-rgpd", 2, "", 22, [1386], [{ e: 1386 }], [{ visible: ["module-rgpd-component", ".modal"] }], [{ waitForThenClick: ["module-rgpd-component", ".buttons > button:nth-of-type(2)"], retry: 3 }], [{ cc: 1387 }], {}], [1, "inmobi", 2, "", 22, [1388], [{ e: 1389 }], [{ v: 1389 }], [{ c: 1390 }], [{ cc: 1391 }], {}], [1, "insided", 0, "", 10, [1392], [{ e: 1393 }], [{ v: 1393 }], [{ c: 1393 }], [{ check: "none", v: 1393 }], {}], [1, "interface-medien-gebaeck", 2, "", 22, [1394], [{ e: 1395 }], [{ v: 1395 }], [{ if: { e: 1396 }, then: [{ k: 1396 }], else: [{ k: 1397 }, { timeout: 3e3, wv: 1398 }, { all: true, optional: true, k: 1399 }, { k: 1400 }] }], [{ timeout: 2e3, check: "none", wv: 1394 }, { cc: 1401 }], {}], [1, "interia", 2, "", 22, [1402], [{ e: 1403 }], [{ v: 1402 }], [{ c: 1403 }, { c: 1404 }], [{ check: "none", timeout: 3e3, wv: 1402 }], {}], [1, "intermedia-cms-fsi", 2, "", 22, [1405], [{ e: 1406 }], [{ v: 1405 }], [{ c: 1407 }], [{ cc: 1408 }], {}], [1, "invision-guest-terms", 1, "", 22, [1409], [{ e: 1410 }], [{ v: 1409 }], [{ h: 1409 }], [], {}], [1, "ionos.de", 2, "", 22, [1411, 1412], [{ e: 1412 }], [{ v: 1412 }], [{ k: 1413 }, { k: 1414 }], [], {}], [1, "islington-council", 2, "", 22, [1415], [{ e: 1416 }], [{ v: 1415 }], [{ timeout: 1e4, w: 1417 }, { c: 1416 }, { c: 1418 }], [{ cc: 1419 }, { cc: 1420 }], {}], [1, "iubenda", 2, "", 22, [1421], [{ e: 1421 }], [{ any: [{ v: 1422 }, { v: 1423 }] }], [{ if: { e: 1424 }, then: [{ k: 1424 }], else: [{ if: { e: 1425 }, then: [{ c: 1425 }, { c: 1426 }, { c: 1427 }], else: [{ if: { e: 1423 }, then: [{ k: 1423 }, { c: 1428 }, { timeout: 2e3, optional: true, c: 1429 }], else: [{ h: 1421 }] }] }] }], [{ any: [{ eval: "EVAL_IUBENDA_1" }, { cc: 1430 }] }], {}], [1, "iWink", 2, "", 22, [1431], [{ e: 1431 }], [{ v: 1431 }], [{ c: 1432 }], [{ cc: 1433 }], {}], [1, "jcookie", 1, "", 22, [1434], [{ e: 1435 }], [{ v: 1436 }], [{ h: 1434 }], [], {}], [1, "jetpack-eu-cookie-law", 1, "", 22, [1437], [{ e: 1437 }], [{ v: 1437 }], [{ h: 1437 }], [], {}], [1, "jobindex", 2, "", 22, [1438], [{ e: 1438 }], [{ v: 1438 }], [{ if: { e: 1439 }, then: [{ all: true, k: 1439 }] }, { c: 1440 }], [{ cc: 594 }], {}], [1, "johnlewis.com", 2, "", 22, [1441], [{ e: 1441 }], [{ e: 1441 }], [{ k: 1442 }, { wait: 500 }, { all: true, optional: true, k: 1443 }, { k: 1444 }], [], {}], [1, "jokul-cookie-consent", 2, "", 22, [], [{ e: 1445 }], [{ v: 1445 }], [{ if: { e: 1446 }, then: [{ c: 1446 }], else: [{ if: { e: 1447 }, then: [{ all: true, k: 1447 }] }, { c: 1445 }] }], [{ cc: 1448 }], {}], [1, "jollyroom", 2, "", 22, [1449], [{ e: 1450 }], [{ v: 1451 }], [{ c: 1452 }, { if: { e: 1453 }, then: [{ all: true, k: 1453 }] }, { retry: 5, retryInterval: 500, c: 1450 }], [{ check: "none", v: 1454 }], {}], [1, "jpmorgan-chase", 2, "", 22, [1455], [{ e: 1456 }], [{ v: 1456 }], [{ c: 1456 }], [{ cc: 1457 }], {}], [1, "jquery-cookiepolicy", 0, "", 10, [1458], [{ e: 1459 }], [{ v: 1458 }], [{ if: { e: 1460 }, then: [{ c: 1460 }], else: [{ k: 1461 }, { wv: 1462 }, { all: true, optional: true, k: 1463 }, { c: 1464 }] }], [{ cc: 1465 }], {}], [1, "jquery.cookieBar", 1, "", 22, [1466], [{ e: 1467 }], [{ check: "any", v: 1467 }], [{ h: 1466 }], [{ check: "none", v: 1467 }, { negated: true, cc: 1468 }], {}], [1, "kamsoft-cookiesmanager", 2, "", 22, [1469], [{ e: 1470 }], [{ v: 1470 }], [{ if: { e: 1471 }, then: [{ c: 1471 }], else: [{ c: 1472 }, { wv: 1473 }, { all: true, optional: true, k: 1474 }, { k: 1473 }] }], [], {}], [1, "kaspersky-cookies-notification", 1, "", 22, [1475], [{ e: 1476 }, { e: 1477 }], [{ v: 1477 }], [{ h: 1478 }], [], {}], [1, "kconsent", 0, "", 10, [1479], [{ e: 1480 }], [{ v: 1481 }], [{ c: 1482 }], [], {}], [1, "ketch", 2, "", 10, [1483, 1484, 1485], [{ any: [{ e: 1483 }, { e: 1484 }, { e: 1485 }, { e: 1486 }] }], [{ any: [{ v: 1483 }, { v: 1484 }, { v: 1485 }] }], [{ if: { e: 1487 }, then: [{ c: 1487 }, { if: { v: 1488 }, then: [{ k: 1489 }] }], else: [{ if: { e: 1490 }, then: [{ c: 1490 }], else: [{ if: { e: 1491 }, then: [{ if: { e: 1492 }, then: [{ c: 1492 }], else: [{ if: { e: 1493 }, then: [{ k: 1493 }] }] }] }] }] }, { timeout: 1e3, optional: true, w: 1494 }, { if: { e: 1494 }, then: [{ if: { e: 1495 }, then: [{ c: 1495 }], else: [{ all: true, optional: true, k: 1496 }] }, { k: 1489 }] }], [{ cc: 1497 }], { intermediate: false }], [1, "klaxit-cookie-consent", 2, "", 22, [1498], [{ e: 1499 }], [{ v: 1500 }], [{ c: 1501 }], [{ cc: 1502 }], {}], [1, "klp-cookie-banner", 2, "", 22, [1503], [{ e: 1503 }], [{ v: 1504 }], [{ c: 1505 }], [{ wait: 500 }, { cc: 1506 }], {}], [1, "kolibri-cookie-widget", 2, "", 22, [1507], [{ e: 1507 }], [{ v: 1507 }], [{ c: 1508 }, { check: "none", timeout: 5e3, wv: 1507 }], [{ cc: 1509 }], {}], [1, "krieger-consent-form", 2, "", 22, [1510], [{ e: 1511 }], [{ v: 1512 }], [{ all: true, optional: true, k: 1513 }, { c: 1514 }], [{ cc: 1515 }], {}], [1, "krown-cookie-banner", 0, "", 22, [1516], [{ e: 1517 }], [{ v: 1517 }], [{ c: 1517 }], [{ wait: 500 }, { eval: "EVAL_KROWN_COOKIE_BANNER_TEST" }], {}], [1, "kulturit-ccm", 0, "", 22, [1518], [{ exists: ["#kit-ccm", ".ccm-container dialog"] }], [{ visible: ["#kit-ccm", ".ccm-container dialog[open] .action-buttons"] }], [{ waitForThenClick: ["#kit-ccm", "dialog[open] .action-buttons button:not(.accept-selected):last-of-type"] }], [{ waitForVisible: ["#kit-ccm", ".ccm-container dialog .action-buttons"], check: "none", timeout: 2e3 }], {}], [1, "kunstmaan-cookie-bar", 2, "", 22, [1519], [{ e: 1519 }], [{ v: 1520 }], [{ c: 1521 }, { wv: 1522 }, { if: { e: 1523 }, then: [{ all: true, k: 1523 }] }, { k: 1522 }], [{ wait: 1e3 }, { cc: 1524 }], {}], [1, "kwik-fit", 0, "", 22, [1525, 1526], [{ e: 1527 }], [{ v: 1528 }], [{ c: 1529 }, { wv: 1530 }, { all: true, optional: true, k: 1531 }, { c: 1532 }, { check: "none", wv: 1525 }], [], {}], [1, "la7-cmp", 2, "", 22, [1533], [{ e: 1534 }], [{ v: 1534 }], [{ c: 1534 }], [{ cc: 1535 }], {}], [1, "lab-network-consent", 2, "", 22, [1536], [{ e: 1537 }], [{ v: 1536 }], [{ c: 1537 }], [{ cc: 1538 }], {}], [1, "landgate", 2, "", 22, [1539], [{ e: 1540 }], [{ v: 1539 }], [{ c: 1541 }], [{ check: "none", timeout: 1e3, wv: 1539 }], {}], [1, "laravel-cc-banner", 2, "", 22, [1542], [{ e: 1543 }], [{ v: 1544 }], [{ c: 1545 }, { wv: 1546 }, { all: true, optional: true, k: 1547 }, { k: 1546 }], [], {}], [1, "laravel-cookie-consent", 2, "", 22, [1548], [{ e: 1548 }], [{ v: 1548 }], [{ c: 1549 }, { wv: 1550 }, { all: true, optional: true, k: 1551 }, { c: 1552 }, { check: "none", wv: 1548 }], [{ cc: 1553 }], {}], [1, "laravel-eu-cookie-consent", 2, "", 22, [1554], [{ e: 1555 }], [{ v: 1554 }], [{ if: { v: 1556 }, then: [{ k: 1556 }], else: [{ all: true, optional: true, k: 1557 }, { c: 1558 }] }], [], {}], [1, "lawwwing", 2, "", 22, [1559], [{ e: 1559 }], [{ v: 1560 }], [{ c: 1561 }, { check: "none", timeout: 2e3, wv: 1562 }], [{ cc: 1563 }], {}], [1, "legally-ok", 2, "", 22, [1564], [{ exists: ["#jmdCookieConsentWrapper", "#cookieConsent"] }], [{ visible: ["#jmdCookieConsentWrapper", "#cookieConsent"] }], [{ waitForThenClick: ["#jmdCookieConsentWrapper", "#cookieConsent .js-select-none"] }], [{ cc: 181 }], {}], [1, "lg-cookiecontrol", 2, "", 22, [1565], [{ e: 1566 }], [{ v: 1565 }], [{ if: { e: 1567 }, then: [{ k: 1567 }], else: [{ c: 1568 }] }], [{ cc: 1569 }], {}], [1, "lia", 2, "", 22, [1570], [{ e: 1570 }], [{ e: 1571 }, { v: 1570 }], [{ c: 1572 }], [], {}], [1, "liferay-cookies-banner", 2, "", 22, [1573], [{ e: 1573 }, { negated: true, e: 1574 }], [{ v: 1573 }], [{ c: 1575 }], [{ cc: 1576 }], {}], [1, "lightbox", 2, "", 22, [1577], [{ e: 1578 }], [{ v: 1578 }], [{ k: 1579 }], [], {}], [1, "lineagrafica", 1, "", 22, [1580], [{ e: 1580 }], [{ e: 1580 }], [{ h: 1580 }], [], {}], [1, "lineagrafica-reject", 2, "", 22, [1581], [{ e: 1582 }], [{ v: 1582 }], [{ c: 1582 }], [{ check: "none", timeout: 2e3, wv: 1583 }], {}], [1, "linkedin.com", 2, "", 22, [1584], [{ e: 1584 }], [{ v: 1584 }], [{ wv: 1585 }, { wait: 500 }, { c: 1585 }], [{ check: "none", wv: 1584 }], {}], [1, "localbiz-cookie-notice", 1, "", 22, [1586], [{ e: 1587 }], [{ v: 1586 }], [{ h: 1586 }], [], {}], [1, "lta", 0, "", 10, [1588], [{ e: 1588 }], [{ v: 1588 }], [{ c: 1589 }, { wv: 1590 }, { all: true, optional: true, k: 1591 }, { k: 1592 }], [{ cc: 1593 }], {}], [1, "macaron", 2, "", 22, [1594], [{ e: 1594 }], [{ v: 1595 }], [{ if: { e: 1596 }, then: [{ k: 1596 }], else: [{ c: 1597 }, { w: 1598 }, { wv: 1599 }, { all: true, optional: true, c: 1600 }, { c: 1601 }] }], [{ cc: 1602 }], {}], [1, "mars-cookies-prompt", 2, "", 22, [1603], [{ e: 1604 }], [{ v: 1604 }], [{ if: { e: 1605 }, then: [{ k: 1606 }] }, { wv: 1607 }, { if: { e: 1608 }, then: [{ all: true, k: 1608 }] }, { k: 1609 }], [{ cc: 1610 }], {}], [1, "mco-consent", 2, "", 22, [1611, 1612, 1613, 1614], [{ e: 1615 }], [{ v: 1616 }], [{ c: 1616 }], [{ cc: 1617 }], {}], [1, "mediamarkt.de", 2, "", 22, [1618, 1619], [{ e: 1620 }], [{ e: 1620 }], [{ k: 1621 }], [], {}], [1, "mediawiki-cookieconsent", 2, "", 22, [1622], [{ e: 1622 }], [{ v: 1623 }], [{ c: 1624 }, { wv: 1625 }, { all: true, optional: true, k: 1626 }, { k: 1625 }], [{ cc: 1627 }], {}], [1, "mensaje-cookies", 2, "", 22, [1628, 1629], [{ e: 1630 }, { any: [{ e: 1631 }, { e: 1632 }] }], [{ v: 1633 }], [{ if: { e: 1631 }, then: [{ c: 1631 }], else: [{ c: 1634 }, { c: 1632 }] }, { check: "none", timeout: 3e3, wv: 1633 }], [{ check: "none", v: 1633 }], {}], [1, "mercedes-benz-cmm", 2, "", 22, [1635], [{ exists: ["cmm-cookie-banner", ".cmm-cookie-banner__wrapper"] }], [{ visible: ["cmm-cookie-banner", ".cmm-cookie-banner__wrapper.visible"] }], [{ if: { exists: ["cmm-cookie-banner", "wb7-button[data-test=handle-deny-button]"] }, then: [{ waitForThenClick: ["cmm-cookie-banner", "wb7-button[data-test=handle-deny-button]"] }], else: [{ waitForThenClick: ["cmm-cookie-banner", "wb7-button[data-test=toggle-cookie-details]"] }, { waitForVisible: ["cmm-cookie-banner", "wb7-button[data-test=handle-confirm-selection-button]"] }, { click: ["cmm-cookie-banner", "input[data-test=handle-category-change]:checked:not([disabled])"], all: true, optional: true }, { waitForThenClick: ["cmm-cookie-banner", "wb7-button[data-test=handle-confirm-selection-button]"] }] }], [{ waitForVisible: ["cmm-cookie-banner", ".cmm-cookie-banner__wrapper.visible"], timeout: 2e3, check: "none" }], {}], [1, "meridiancu", 2, "", 22, [1636], [{ e: 1637 }], [{ v: 1638 }], [{ c: 1639 }, { wv: 1640 }, { optional: true, k: 1641 }, { c: 1642 }], [{ cc: 1643 }], {}], [1, "meta-cookie-dialog", 2, "", 22, [1644, 1645], [{ e: 1646 }], [{ v: 1646 }], [{ retry: 3, retryInterval: 500, c: 1647 }, { check: "none", wv: 1647 }], [], {}], [1, "meteored", 2, "", 22, [1648], [{ e: 1649 }], [{ v: 1649 }], [{ c: 1650 }, { retry: 5, retryInterval: 500, c: 1651 }], [{ timeout: 1e3, check: "none", wv: 1648 }, { cc: 1652 }], {}], [1, "microsoft.com", 2, "", 22, [1653], [{ e: 1653 }], [{ e: 1653 }], [{ eval: "EVAL_MICROSOFT_0" }], [{ eval: "EVAL_MICROSOFT_2" }], {}], [2, "milestone-internet", 1, "", 22, [1654], [{ e: 1655 }], [{ v: 1655 }], [{ h: 1654 }, { removeClass: "pushItDown", selector: ".pushItDown", optional: true }], [], {}], [1, "minitool", 1, "", 22, [1656], [{ e: 1657 }], [{ v: 1656 }], [{ h: 1656 }], [{ check: "none", v: 1656 }], {}], [1, "mirasvit-gdpr", 2, "", 22, [1658, 1659, 1660, 1661], [{ e: 1662 }], [{ check: "any", v: 1662 }], [{ if: { check: "any", v: 1663 }, then: [{ any: [{ k: 1664 }, { k: 1665 }, { k: 1666 }] }], else: [{ any: [{ k: 1667 }, { k: 1668 }] }, { check: "any", timeout: 5e3, wv: 1669 }, { all: true, timeout: 2e3, optional: true, c: 1670 }, { any: [{ k: 1671 }, { k: 1672 }, { k: 1673 }] }] }, { check: "none", timeout: 1e4, wv: 1662 }], [{ cc: 1674 }, { check: "none", v: 1662 }], {}], [1, "miteco", 2, "", 22, [1675, 1676], [{ e: 1677 }], [{ v: 1678 }], [{ c: 1677 }, { wv: 1679 }, { if: { e: 1680 }, then: [{ k: 1681 }] }, { k: 1679 }], [], {}], [1, "mittanbud", 0, "", 22, [], [{ e: 1682 }], [{ v: 1683 }], [{ c: 1684 }], [{ cc: 1685 }, { cc: 277 }], {}], [1, "mkdocs-material", 2, "", 22, [1686, 1687], [{ e: 1688 }], [{ v: 1689 }], [{ all: true, optional: true, k: 1690 }, { c: 1691 }], [], {}], [1, "mobirise-cookies-alert", 2, "", 22, [1692], [{ e: 1693 }], [{ v: 1692 }], [{ retry: 3, retryInterval: 1500, c: 1693 }], [{ check: "none", timeout: 4e3, wv: 1692 }], {}], [1, "mobirise-cookies-alert-cosmetic", 1, "", 22, [1692], [{ e: 1692 }, { negated: true, e: 1693 }], [{ v: 1692 }], [{ h: 1692 }], [], {}], [1, "moebel-de-group", 2, "", 22, [1694], [{ e: 1695 }], [{ v: 1695 }], [{ retry: 2, retryInterval: 3500, c: 1695 }], [{ timeout: 4e3, check: "none", wv: 1695 }], {}], [1, "moneysavingexpert.com", 2, "", 22, [], [{ e: 1696 }], [{ v: 1696 }], [{ k: 1697 }, { k: 1698 }], [], {}], [1, "Moove", 2, "", 22, [1699], [{ e: 1699 }], [{ v: 1700 }], [{ if: { e: 1701 }, then: [{ k: 1701 }], else: [{ if: { e: 1702 }, then: [{ k: 1702 }, { wv: 1703 }, { eval: "EVAL_MOOVE_0" }, { k: 1704 }], else: [{ h: 1699 }] }] }], [{ check: "none", v: 1699 }], {}], [1, "msg-cookie", 1, "", 22, [1705], [{ e: 1706 }], [{ v: 1706 }], [{ h: 1707 }], [], {}], [1, "musti", 2, "", 22, [1708], [{ e: 1709 }], [{ v: 1708 }], [{ c: 1710 }], [{ check: "none", timeout: 2e3, wv: 1708 }], {}], [1, "ncsoft-cookie-toast", 2, "", 22, [1711, 1712], [{ e: 1711 }], [{ v: 1713 }], [{ c: 1713 }], [{ cc: 1714 }], {}], [1, "nhs.uk", 2, "", 22, [1715], [{ e: 1715 }], [{ e: 1715 }], [{ k: 1716 }], [], {}], [1, "nikkeibp", 2, "", 22, [1717], [{ e: 1718 }], [{ v: 1718 }], [{ c: 1718 }], [{ timeout: 1e3, check: "none", wv: 1717 }], {}], [1, "nnk-theme-cookie-bar", 2, "", 22, [1719], [{ e: 1720 }], [{ v: 1721 }], [{ c: 1720 }], [{ cc: 1722 }], {}], [1, "no-transit-consent", 2, "", 22, [1723], [{ e: 1723 }], [{ v: 1723 }], [{ c: 1724 }], [{ cc: 1725 }], {}], [1, "nowaera", 2, "", 22, [1726, 1727], [{ e: 1728 }], [{ v: 1729 }], [{ c: 1730 }, { wv: 1731 }, { all: true, optional: true, k: 1732 }, { k: 1731 }], [{ cc: 1733 }, { cc: 1734 }], {}], [1, "npo-ccm", 2, "", 22, [1735], [{ any: [{ exists: ["ccm-notification", "#ccm_notification .ccm_btn--save"] }, { exists: ["ccm-notification", "#ccm_notification #ccm_close"] }] }], [{ visible: ["ccm-notification", "#ccm_notification"] }], [{ if: { exists: ["ccm-notification", "#ccm_notification .ccm_btn--deny-all"] }, then: [{ click: ["ccm-notification", "#ccm_notification .ccm_btn--deny-all"] }], else: [{ if: { exists: ["ccm-notification", "#ccm_notification .ccm_btn--save"] }, then: [{ click: ["ccm-notification", "#ccm_notification input[type='radio'][value='false']"], all: true, optional: true }, { waitForThenClick: ["ccm-notification", "#ccm_notification .ccm_btn--save:not(.ccm_btn_disabled)"] }], else: [{ waitForThenClick: ["ccm-notification", "#ccm_notification #ccm_close"] }] }] }], [], {}], [1, "nuxt-cookie-manager", 2, "", 22, [1736], [{ e: 1737 }], [{ v: 1736 }], [{ if: { e: 1738 }, then: [{ c: 1738 }], else: [{ all: true, optional: true, k: 1739 }, { c: 1740 }] }], [{ check: "none", timeout: 2e3, wv: 1736 }], {}], [1, "nyu-langone", 2, "", 22, [1741], [{ e: 1742 }], [{ v: 1743 }], [{ c: 1744 }], [{ cc: 1745 }], {}], [1, "obi.de", 2, "", 22, [1746], [{ e: 1747 }], [{ v: 1747 }], [{ k: 1748 }], [], {}], [1, "occ-gdpr", 2, "", 22, [1749], [{ e: 1750 }], [{ v: 1751 }], [{ c: 1752 }, { wv: 1753 }, { if: { e: 1754 }, then: [{ all: true, k: 1754 }] }, { k: 1753 }], [{ cc: 1755 }], {}], [1, "om", 2, "", 22, [1756, 1757], [{ e: 1758 }], [{ e: 1758 }], [{ if: { e: 1759 }, then: [{ c: 1759 }], else: [{ all: true, optional: true, k: 1760 }, { c: 1761 }] }], [], {}], [1, "onecom-cookie-banner", 1, "", 22, [1762], [{ e: 1763 }], [{ v: 1762 }], [{ h: 1762 }], [], {}], [1, "opencmp", 2, "", 22, [], [{ exists: [".cmp-root-container", "#cmp_consentDialog .cmp_ui .cmp_mainButtons"] }], [{ visible: [".cmp-root-container", "#cmp_consentDialog .cmp_ui"] }], [{ if: { exists: [".cmp-root-container", ".cmp_ui .cmp_mainButtons .cmp_button:not(.cmp-button-accept-all)"] }, then: [{ click: [".cmp-root-container", ".cmp_ui .cmp_mainButtons .cmp_button:not(.cmp-button-accept-all)"] }], else: [{ click: [".cmp-root-container", ".cmp_ui .cmp_mainButtons .cmp_saveLink a"] }, { waitFor: [".cmp-root-container", ".cmp_ui .cmp_level1Container"], timeout: 3e3 }, { click: [".cmp-root-container", ".cmp_ui .cmp_activateAll button:nth-child(2)"], all: true, optional: true }, { click: [".cmp-root-container", ".cmp_ui .cmp_level1Container .cmp_levelItem:nth-child(2) > div"], optional: true }, { waitFor: [".cmp-root-container", ".cmp_ui .cmp_level1Container .cmp_levelItem.cmp_active:nth-child(2)"], timeout: 2e3, optional: true }, { click: [".cmp-root-container", ".cmp_ui .cmp_activateAll button:nth-child(2)"], all: true, optional: true }, { click: [".cmp-root-container", ".cmp_ui .cmp_mainButtons .cmp_saveLink a"] }] }], [{ waitForVisible: [".cmp-root-container", "#cmp_consentDialog .cmp_ui"], timeout: 2e3, check: "none" }], {}], [1, "openli", 2, "", 22, [1764], [{ e: 1764 }], [{ check: "any", v: 1765 }], [{ c: 1766 }], [], {}], [1, "optum-legal-banner", 2, "", 22, [1767], [{ e: 1768 }], [{ v: 1767 }], [{ c: 1768 }], [{ check: "none", timeout: 2e3, wv: 1767 }], {}], [1, "orejime", 2, "", 22, [1769, 1770, 1771], [{ e: 1772 }], [{ v: 1772 }], [{ if: { e: 1773 }, then: [{ c: 1774 }], else: [{ optional: true, timeout: 1e3, c: 1775 }, { c: 1776 }] }], [{ check: "none", timeout: 2e3, wv: 1772 }], {}], [2, "orw-cookie-banner", 1, "", 22, [1777], [{ e: 1777 }], [{ v: 1778 }], [{ h: 1777 }, { removeClass: "cookie-not-accepted", selector: "body", optional: true }, { removeClass: "drawer-opt", selector: "body", optional: true }, { removeClass: "show-overlay", selector: "body", optional: true }], [], {}], [1, "osano", 2, "", 22, [1779], [{ e: 1780 }], [{ eval: "EVAL_OSANO_DETECT" }, { v: 1781 }], [{ if: { e: 1782 }, then: [{ k: 1782 }], else: [{ if: { v: 1783 }, then: [{ k: 1783 }], else: [{ h: 1779 }] }] }], [], {}], [1, "otto.de", 2, "", 22, [1784], [{ e: 1784 }], [{ v: 1785 }], [{ k: 1786 }], [], {}], [1, "overleaf", 2, "", 22, [1787], [{ e: 1788 }], [{ v: 1788 }], [{ c: 1789 }], [{ cc: 1790 }], {}], [1, "pabcogypsum", 2, "", 22, [1791], [{ e: 1792 }], [{ v: 1792 }], [{ c: 1793 }], [], {}], [1, "pandectes", 2, "", 22, [1794, 1795], [{ any: [{ e: 1794 }, { exists: ["pandectes-cmp", "dialog.notice"] }] }], [{ any: [{ v: 1794 }, { visible: ["pandectes-cmp", "dialog.notice"] }] }], [{ if: { e: 1794 }, then: [{ if: { v: 1796 }, then: [{ c: 1796 }], else: [{ c: 1797 }, { wv: 1798 }, { k: 1799 }, { k: 1798 }] }, { optional: true, h: 1794 }], else: [{ if: { visible: ["pandectes-cmp", "dialog.notice button.deny"] }, then: [{ waitForThenClick: ["pandectes-cmp", "dialog.notice button.deny"] }], else: [{ waitForThenClick: ["pandectes-cmp", "dialog.notice button.preferences"] }, { waitForVisible: ["pandectes-cmp", "dialog.preferences"] }, { if: { exists: ["pandectes-cmp", "dialog.preferences button.deny"] }, then: [{ click: ["pandectes-cmp", "dialog.preferences button.deny"] }], else: [{ click: ["pandectes-cmp", "dialog.preferences button[role=switch][aria-checked=true]:not([disabled])"], all: true, optional: true }, { click: ["pandectes-cmp", "dialog.preferences button.custom"] }] }] }] }], [{ wait: 500 }, { eval: "EVAL_PANDECTES_TEST" }], {}], [1, "passle", 2, "", 22, [1800], [{ e: 1801 }], [{ v: 1802 }], [{ c: 1803 }], [{ wait: 1e3 }, { cc: 1804 }], {}], [1, "paypal-us", 2, "", 22, [1805], [{ e: 1806 }], [{ v: 1806 }], [{ if: { e: 1807 }, then: [{ retry: 10, retryInterval: 500, c: 1807 }], else: [{ h: 1806 }] }], [{ wait: 1e3 }, { cc: 1808 }], {}], [1, "paypal.com", 2, "", 22, [1809], [{ e: 1809 }], [{ v: 1810 }], [{ retry: 10, retryInterval: 500, c: 1807 }], [{ wait: 500 }, { cc: 1808 }], {}], [1, "paytronix", 2, "", 22, [1811], [{ e: 1812 }], [{ v: 1811 }], [{ c: 1813 }], [{ cc: 1814 }], {}], [1, "pe-cookies", 2, "", 22, [1815], [{ exists: ["pe-cookies", "#pecookies-continue-btn"] }], [{ visible: ["pe-cookies", ".pecookies__modal"] }], [{ waitForThenClick: ["pe-cookies", "#pecookies-continue-btn"] }], [{ cc: 1816 }], {}], [1, "pigu", 2, "", 22, [], [{ e: 1817 }], [{ v: 1818 }], [{ retry: 10, retryInterval: 500, c: 1818 }], [{ check: "none", timeout: 2e3, wv: 1818 }], {}], [1, "pikpak", 2, "", 22, [1819], [{ e: 1820 }], [{ v: 1821 }], [{ c: 1822 }, { wv: 1823 }, { all: true, optional: true, k: 1824 }, { c: 1825 }], [{ any: [{ cc: 1826 }, { cc: 1827 }] }], {}], [1, "piwik-pro", 2, "", 22, [1828, 1829, 1830], [{ e: 1831 }], [{ check: "any", v: 1832 }], [{ if: { e: 1833 }, then: [{ c: 1833 }, { timeout: 1e3, optional: true, c: 1834 }], else: [{ if: { v: 1835 }, then: [{ k: 1835 }], else: [{ c: 1836 }, { timeout: 2e3, wv: 1837 }, { if: { v: 1835 }, then: [{ k: 1835 }], else: [{ all: true, optional: true, k: 1838 }, { if: { v: 1839 }, then: [{ k: 1839 }], else: [{ k: 1836 }] }] }] }] }], [{ cc: 1733 }, { cc: 1840 }], {}], [1, "pl-bip-cookies", 1, "", 22, [1841], [{ e: 1842 }], [{ v: 1843 }], [{ h: 1843 }], [], {}], [1, "planned-parenthood", 2, "", 22, [1844], [{ e: 1845 }, { e: 1846 }], [{ v: 1844 }], [{ c: 1845 }, { wv: 1847 }, { c: 1846 }], [{ cc: 1848 }], {}], [1, "plentyshop", 2, "", 22, [1849], [{ e: 1850 }], [{ check: "any", v: 1851 }], [{ if: { e: 1852 }, then: [{ c: 1852 }], else: [{ c: 1853 }] }], [{ check: "none", v: 1851 }, { cc: 1854 }], {}], [1, "plumrocket-cookie-consent", 2, "", 22, [], [{ e: 1855 }], [{ v: 1855 }, { negated: true, e: 1856 }], [{ if: { e: 1857 }, then: [{ c: 1857 }], else: [{ c: 1858 }, { c: 1859 }] }], [{ cc: 1860 }], {}], [1, "pmc", 1, "", 22, [1861], [{ e: 1861 }], [{ v: 1861 }], [{ h: 1861 }], [], {}], [1, "pmc-privacy-banner", 2, "", 22, [1862], [{ e: 1862 }], [{ v: 1862 }], [{ c: 1863 }], [{ check: "none", v: 1862 }], {}], [1, "police-uk", 2, "", 22, [1864], [{ e: 1865 }], [{ v: 1865 }], [{ c: 1865 }, { timeout: 5e3, optional: true, wv: 1866 }, { optional: true, k: 1866 }], [{ cc: 1867 }], {}], [1, "polisen-se", 0, "", 10, [1868], [{ e: 1869 }], [{ v: 1868 }], [{ c: 1870 }], [{ check: "none", timeout: 3e3, optional: true, wv: 1868 }, { cc: 1871 }], {}], [1, "popupsmart-cookie-notice", 1, "", 22, [1872], [{ e: 1873 }], [{ v: 1874 }], [{ h: 1872 }], [], {}], [1, "pornhat", 1, "", 22, [1875], [{ v: 1876 }], [{ v: 1876 }], [{ h: 1875 }], [], {}], [1, "prestashop-advanced-cookie-banner", 2, "", 22, [1877], [{ e: 1878 }], [{ check: "any", v: 1877 }], [{ if: { check: "any", v: 1879 }, then: [{ c: 1879 }], else: [{ all: true, optional: true, k: 1880 }, { c: 1881 }] }], [{ cc: 1882 }], {}], [1, "pride.com", 1, "", 22, [1883], [{ e: 1884 }], [{ v: 1884 }], [{ h: 1883 }], [], {}], [1, "PrimeBox CookieBar", 2, "", 22, [617], [{ e: 1885 }], [{ check: "any", v: 1885 }], [{ optional: true, k: 1886 }, { h: 617 }], [{ negated: true, cc: 1887 }], {}], [1, "primeconsent", 2, "", 22, [1888], [{ exists: [".azpp-cookie-consent", ".banner-main"] }], [{ visible: [".azpp-cookie-consent", ".banner-main .button-group > button"] }], [{ waitForThenClick: [".azpp-cookie-consent", ".banner-main .button-group > button:nth-of-type(2):nth-last-of-type(2)"] }], [{ any: [{ cc: 1889 }, { cc: 1890 }] }], {}], [1, "privacy-area", 0, "", 22, [1891], [{ e: 1892 }, { e: 1893 }], [{ v: 1891 }], [{ if: { e: 1894 }, then: [{ k: 1894 }], else: [{ all: true, optional: true, k: 1895 }, { k: 1893 }] }, { check: "none", timeout: 3e3, wv: 1891 }], [{ cc: 1896 }], {}], [1, "privacylab-elmo", 2, "", 22, [1897], [{ e: 1898 }], [{ v: 1899 }], [{ c: 1900 }], [{ wait: 500 }, { cc: 1901 }], {}], [1, "privado", 0, "", 10, [312], [{ e: 1902 }], [{ v: 1902 }], [{ if: { e: 1903 }, then: [{ c: 1903 }], else: [{ c: 1904 }, { wv: 1905 }, { c: 1905 }] }], [], {}], [1, "pro-euclp-cookies-box", 1, "", 22, [1906], [{ e: 1907 }], [{ v: 1906 }], [{ h: 1906 }], [{ check: "none", v: 1906 }], {}], [1, "pubfactory", 0, "", 22, [1908], [{ e: 1908 }], [{ v: 1908 }], [{ timeout: 2e4, optional: true, w: 1909 }, { c: 1910 }, { check: "none", wv: 1908 }], [{ cc: 1911 }], {}], [1, "pubtech", 2, "", 22, [1912], [{ e: 1912 }], [{ v: 1913 }], [{ if: { e: 1914 }, then: [{ k: 1914 }], else: [{ k: 1915 }] }], [{ eval: "EVAL_PUBTECH_0" }], {}], [1, "purecookie-cosmetic", 1, "", 22, [1916], [{ e: 1917 }], [{ v: 1916 }], [{ h: 1916 }], [{ check: "none", v: 1916 }], {}], [1, "qtip-jgrowl-cookies", 2, "", 22, [], [{ e: 1918 }], [{ v: 1918 }], [{ c: 1919 }], [{ check: "none", timeout: 2e3, wv: 1918 }], {}], [1, "quantcast", 2, "", 22, [1920], [{ e: 1921 }], [{ v: 1922 }], [{ if: { e: 1923 }, then: [{ timeout: 2e3, optional: true, w: 1924 }, { all: true, optional: true, k: 1925 }, { timeout: 2e3, optional: true, w: 1926 }, { all: true, optional: true, k: 1927 }, { timeout: 2e3, c: 1928 }, { timeout: 2e3, optional: true, c: 1929 }], else: [{ if: { e: 1930 }, then: [{ timeout: 5e3, c: 1931 }], else: [{ timeout: 2e3, w: 1932 }, { if: { e: 1933 }, then: [{ k: 1933 }], else: [{ k: 1934 }, { wv: 1935 }, { all: true, optional: true, k: 1936 }, { click: [".qc-cmp2-main", "xpath///button[contains(., 'REJECT ALL') or contains(., 'ALLE VERWERPEN') or contains(., '\u0391\u03A0\u039F\u03A1\u03A1\u0399\u03A0\u03A4\u03A9 \u03A4\u0391 \u03A0\u0391\u039D\u03A4\u0391') or contains(., 'RESPINGERE TOTAL\u0102') or contains(., 'ALLE ABLEHNEN') or contains(., 'ODRZUCENIE') or contains(., 'BLOQUEAR TODO') or contains(., 'REJEITAR TODOS') or contains(., 'RIFIUTA TUTTO') or contains(., 'TOUT REFUSER') or contains(., '\u041E\u0422\u041A\u041B\u041E\u041D\u0418\u0422\u042C \u0412\u0421\u0415\u0425')]"], optional: true }, { check: "none", timeout: 1e3, optional: true, wv: 1937 }, { if: { v: 1937 }, then: [{ if: { e: 1938 }, then: [{ k: 1938 }], else: [{ waitForThenClick: [".qc-cmp2-main", "xpath///button[contains(.,'SAVE & EXIT') or contains(.,'SALVA ED ESCI') or contains(.,'GUARDAR Y SALIR') or contains(.,'SPEICHERN & VERLASSEN')"], timeout: 5e3 }] }] }] }] }] }], [], {}], [1, "quick-cookie-consent", 2, "", 22, [1939], [{ e: 1940 }], [{ v: 1940 }], [{ c: 1940 }, { wv: 1941 }, { if: { e: 1942 }, then: [{ k: 1943 }] }, { if: { e: 1944 }, then: [{ k: 1944 }], else: [{ k: 1945 }] }], [{ cc: 1946 }, { negated: true, cc: 1947 }, { negated: true, cc: 1948 }, { negated: true, cc: 1949 }], {}], [1, "r42-consent-iframe", 0, "", 10, [], [{ exists: ["#consent_iframe", ".stsp__cookie-container #declineButton"] }], [{ v: 1950 }, { visible: ["#consent_iframe", ".stsp__cookie-container #declineButton"] }], [{ waitForThenClick: ["#consent_iframe", ".stsp__cookie-container #declineButton"] }], [{ cc: 1951 }], {}], [1, "r42-cookiebar", 0, "", 10, [1952], [{ e: 1953 }], [{ v: 1954 }], [{ c: 1953 }], [{ timeout: 1e3, check: "none", wv: 1953 }], {}], [1, "r42-st-popup", 0, "", 10, [], [{ e: 1955 }], [{ v: 1955 }], [{ c: 1955 }], [{ timeout: 1e3, check: "none", wv: 1956 }], {}], [1, "rabobank", 2, "", 22, [1957], [{ e: 1957 }], [{ v: 1957 }], [{ if: { e: 1958 }, then: [{ retry: 10, retryInterval: 500, c: 1958 }], else: [{ retry: 10, retryInterval: 500, c: 1959 }] }], [{ cc: 1960 }], {}], [1, "radboud-cookie-overlay", 2, "", 22, [], [{ e: 1961 }], [{ v: 1962 }], [{ c: 1963 }], [{ cc: 1964 }], {}], [1, "radio-canada", 1, "", 22, [1965], [{ e: 1965 }], [{ check: "any", v: 1965 }], [{ h: 1965 }], [], {}], [2, "rakshaos", 0, "", 22, [1966, 1967], [{ e: 1966 }], [{ v: 1966 }], [{ if: { e: 1968 }, then: [{ c: 1968 }], else: [{ c: 1969 }, { c: 1970 }, { addStyle: "display: none !important", selector: "#raksha-consent-banner" }] }], [], {}], [1, "rambler", 2, "", 22, [1971], [{ e: 1971 }], [{ v: 1971 }], [{ c: 1972 }], [{ cc: 1973 }], {}], [1, "rcs-cpmt", 2, "", 22, [1974], [{ exists: ["#_cpmt-iframe", "#_cpmt"] }], [{ v: 1974 }, { visible: ["#_cpmt-iframe", "#_cpmt"] }], [{ waitForThenClick: ["#_cpmt-iframe", "#_cpmt-reject"] }], [], {}], [1, "rdc-concents", 2, "", 22, [1975, 1976], [{ e: 1977 }], [{ v: 1977 }], [{ c: 1977 }], [{ negated: true, e: 1978 }], {}], [1, "react-cookie-consent", 2, "", 22, [], [{ e: 1979 }], [{ v: 1979 }], [{ c: 1979 }], [{ timeout: 1e3, check: "none", wv: 1979 }], {}], [1, "react-cookie-consent-cosmetic", 1, "", 22, [1980], [{ v: 1981 }, { negated: true, e: 1979 }, { negated: true, e: 1982 }], [{ v: 1981 }], [{ h: 1983 }, { optional: true, h: 1984 }], [], {}], [1, "real-cookie-banner", 0, "", 10, [1985], [{ e: 1986 }], [{ v: 1986 }], [{ waitForThenClick: ['div[consent-skip-blocker="1"][id][data-bg] > dialog > div > div > div > div > div > a[role=button]:not([id])', "xpath///span[contains(., ' ohne ') or contains(., 'without') or contains(., 'Ablehnen')]"] }], [{ timeout: 1e3, check: "none", wv: 1986 }], {}], [1, "remove-cookie-notice", 2, "", 22, [1987], [{ e: 1988 }], [{ v: 1989 }], [{ c: 1988 }, { check: "none", timeout: 2e3, wv: 1990 }], [{ check: "none", v: 1990 }], {}], [1, "reviq", 2, "", 22, [], [{ exists: ["[data-reviq-cmp=host]", "[data-reviq-cmp=banner]"] }], [{ visible: ["[data-reviq-cmp=host]", "[data-reviq-cmp=first-layer] [data-reviq-cmp=banner]"] }], [{ if: { visible: ["[data-reviq-cmp=host]", "[data-reviq-cmp=banner] [data-cmp-action=reject]"] }, then: [{ click: ["[data-reviq-cmp=host]", "[data-reviq-cmp=banner] [data-cmp-action=reject]"] }], else: [{ click: ["[data-reviq-cmp=host]", "[data-reviq-cmp=banner] [data-cmp-action=customize]"] }, { waitForThenClick: ["[data-reviq-cmp=host]", "[data-reviq-cmp=overlay] [data-cmp-action=reject]"] }] }], [{ waitForVisible: ["[data-reviq-cmp=host]", "[data-reviq-cmp=first-layer] [data-reviq-cmp=banner]"], check: "none", timeout: 2e3 }], {}], [1, "rijksoverheid-notification-bar", 0, "", 10, [1991], [{ e: 1992 }], [{ v: 1993 }], [{ c: 1994 }], [{ cc: 1995 }], {}], [1, "ring", 2, "", 22, [], [{ e: 1996 }], [{ v: 1997 }], [{ c: 1996 }, { wv: 1998 }, { all: true, optional: true, k: 1999 }, { c: 2e3 }], [], {}], [1, "rws-app", 2, "", 22, [2001], [{ e: 2001 }], [{ v: 2001 }], [{ c: 2002 }], [{ cc: 2003 }], {}], [1, "sachsen-cookiebar", 2, "", 22, [2004], [{ e: 2005 }], [{ v: 2006 }], [{ c: 2007 }], [{ e: 2008 }], {}], [1, "sandstudio-cookie-tip", 2, "", 22, [2009], [{ e: 2010 }], [{ v: 2009 }], [{ c: 2010 }], [{ cc: 2011 }], {}], [1, "sankei-digital", 0, "", 10, [2012], [{ e: 2013 }], [{ v: 2013 }], [{ c: 2013 }], [{ cc: 2014 }], {}], [1, "sas", 2, "", 22, [2015], [{ e: 2016 }], [{ v: 2017 }], [{ c: 2016 }], [{ cc: 594 }], {}], [1, "savills", 2, "", 22, [2018], [{ e: 2018 }], [{ v: 2019 }], [{ c: 2020 }, { wv: 2021 }, { if: { e: 2022 }, then: [{ k: 2023 }] }, { if: { e: 2024 }, then: [{ k: 2025 }] }, { k: 2026 }], [{ cc: 802 }], {}], [1, "sc-cookie-banner", 2, "", 22, [2027], [{ e: 2028 }], [{ v: 2029 }], [{ c: 2028 }], [{ cc: 2030 }], {}], [1, "scottish-government-design-system", 2, "", 22, [2031], [{ e: 2032 }], [{ v: 2032 }], [{ c: 2032 }], [{ cc: 2033 }], {}], [1, "securiti-gpc-flyout", 2, "", 22, [2034], [{ e: 2035 }], [{ v: 2034 }, { negated: true, e: 2036 }], [{ retry: 2, c: 2037 }], [{ check: "none", v: 2038 }], {}], [1, "sedlak-gdpr-cookie", 2, "", 22, [2039], [{ e: 2040 }], [{ v: 2040 }], [{ c: 2040 }], [{ cc: 2041 }], {}], [1, "setupad", 2, "", 22, [2042], [{ e: 2043 }], [{ v: 2043 }], [{ if: { e: 2044 }, then: [{ c: 2044 }], else: [{ c: 2045 }, { wv: 2046 }, { all: true, optional: true, k: 2047 }, { c: 2044 }] }, { check: "none", wv: 2042 }], [{ cc: 2048 }], {}], [1, "sg-cookies", 2, "", 22, [2049], [{ e: 2049 }], [{ v: 2050 }, { v: 2049 }, { e: 2051 }], [{ if: { e: 2052 }, then: [{ c: 2052 }, { check: "none", timeout: 2e3, wv: 2049 }], else: [{ if: { e: 2053 }, then: [{ k: 2053 }, { check: "none", timeout: 2e3, wv: 2049 }], else: [{ all: true, optional: true, k: 2054 }, { k: 2055 }] }] }], [{ cc: 2056 }, { cc: 2057 }], {}], [1, "shoper", 2, "", 22, [2058], [{ e: 2059 }], [{ v: 2059 }], [{ c: 2059 }], [{ check: "none", timeout: 1e3, wv: 2059 }], {}], [1, "shopgold", 2, "", 22, [2060, 2061], [{ e: 2062 }], [{ any: [{ v: 2063 }, { v: 2064 }] }], [{ if: { v: 2063 }, then: [{ if: { e: 2065 }, then: [{ c: 2065 }], else: [{ k: 2066 }, { wv: 2061 }, { all: true, optional: true, k: 2067 }, { k: 2068 }] }], else: [{ c: 2064 }] }], [{ cc: 2069 }], {}], [1, "shopify", 0, "", 22, [2070], [{ e: 2070 }], [{ v: 2070 }], [{ c: 2071 }], [{ eval: "EVAL_SHOPIFY_TEST" }], {}], [1, "shopify-dux", 0, "", 22, [2072], [{ e: 2073 }], [{ v: 2074 }], [{ c: 2073 }], [{ timeout: 2e3, w: 2075 }], {}], [1, "shopware", 2, "", 22, [2076], [{ e: 2077 }], [{ v: 2076 }], [{ if: { e: 2078 }, then: [{ c: 2079 }], else: [{ c: 2080 }, { c: 2081 }] }], [{ timeout: 2e3, check: "none", wv: 2076 }, { wait: 500 }, { cc: 2082 }], {}], [1, "shouldianswer", 1, "", 10, [2083], [{ e: 2083 }], [{ v: 2084 }], [{ h: 2083 }], [], {}], [1, "shueisha-gigaviewer", 2, "", 22, [2085], [{ e: 2086 }], [{ v: 2087 }], [{ c: 2088 }], [{ timeout: 1e3, check: "none", wv: 2085 }], {}], [1, "shueisha-hpplus", 2, "", 22, [2089], [{ e: 2090 }], [{ check: "any", v: 2089 }], [{ c: 2090 }], [{ timeout: 2e3, check: "none", wv: 2089 }], {}], [1, "sibbo", 2, "", 22, [2091], [{ e: 2091 }], [{ v: 2092 }], [{ k: 2092 }], [], {}], [1, "silktide", 2, "", 22, [2093, 2094], [{ e: 2093 }], [{ v: 2093 }], [{ if: { e: 2095 }, then: [{ k: 2095 }], else: [{ k: 2096 }, { c: 2097 }] }], [{ check: "none", timeout: 2e3, wv: 2093 }], {}], [1, "simpleview-cookie-banner", 1, "", 22, [2098, 2099], [{ e: 2100 }], [{ check: "any", v: 2101 }], [{ h: 2102 }], [], {}], [1, "Sirdata", 0, "", 22, [2103], [{ e: 2103 }], [{ check: "any", v: 2104 }], [{ if: { exists: ["#sd-cmp", "xpath///button[contains(., 'Do not accept') or contains(., 'Acceptera inte') or contains(., 'No aceptar') or contains(., 'Ikke acceptere') or contains(., 'Nicht akzeptieren') or contains(., '\u041D\u0435 \u043F\u0440\u0438\u0435\u043C\u0430\u043C') or contains(., '\u039D\u03B1 \u03BC\u03B7\u03BD \u03B3\u03AF\u03BD\u03B5\u03B9 \u03B1\u03C0\u03BF\u03B4\u03BF\u03C7\u03AE') or contains(., 'Niet accepteren') or contains(., 'Nep\u0159ij\xEDmat') or contains(., 'Nie akceptuj') or contains(., 'Nu accepta\u021Bi') or contains(., 'N\xE3o aceitar') or contains(., 'Continuer sans accepter') or contains(., 'Non accettare') or contains(., 'Nem fogad el')]"] }, then: [{ waitForThenClick: ["#sd-cmp", "xpath///button[contains(., 'Do not accept') or contains(., 'Acceptera inte') or contains(., 'No aceptar') or contains(., 'Ikke acceptere') or contains(., 'Nicht akzeptieren') or contains(., '\u041D\u0435 \u043F\u0440\u0438\u0435\u043C\u0430\u043C') or contains(., '\u039D\u03B1 \u03BC\u03B7\u03BD \u03B3\u03AF\u03BD\u03B5\u03B9 \u03B1\u03C0\u03BF\u03B4\u03BF\u03C7\u03AE') or contains(., 'Niet accepteren') or contains(., 'Nep\u0159ij\xEDmat') or contains(., 'Nie akceptuj') or contains(., 'Nu accepta\u021Bi') or contains(., 'N\xE3o aceitar') or contains(., 'Continuer sans accepter') or contains(., 'Non accettare') or contains(., 'Nem fogad el')]"] }], else: [{ if: { exists: ["#sd-cmp", "xpath///span[contains(., '\u0417\u0430\u0434\u0430\u0439\u0442\u0435 \u0432\u0430\u0448\u0438\u0442\u0435 \u0438\u0437\u0431\u043E\u0440\u0438') or contains(., 'Nastavit va\u0161e volby') or contains(., 'Angiv dine valg') or contains(., 'Ihre Auswahl treffen') or contains(., '\u039F\u03C1\u03AF\u03C3\u03C4\u03B5 \u03C4\u03B9\u03C2 \u03B5\u03C0\u03B9\u03BB\u03BF\u03B3\u03AD\u03C2 \u03C3\u03B1\u03C2') or contains(., 'Set your choices') or contains(., 'Establecer preferencias') or contains(., 'Param\xE9trer vos choix') or contains(., 'V\xE1lassza ki a be\xE1ll\xEDt\xE1sokat') or contains(., 'Imposta le tue scelte') or contains(., 'Stel uw keuzes in') or contains(., 'Ustaw swoje wybory') or contains(., 'Definir suas escolhas') or contains(., 'Seta\u021Bi-v\u0103 op\u021Biunile') or contains(., 'St\xE4ll in dina val')]"] }, then: [{ waitForThenClick: ["#sd-cmp", "xpath///span[contains(., '\u0417\u0430\u0434\u0430\u0439\u0442\u0435 \u0432\u0430\u0448\u0438\u0442\u0435 \u0438\u0437\u0431\u043E\u0440\u0438') or contains(., 'Nastavit va\u0161e volby') or contains(., 'Angiv dine valg') or contains(., 'Ihre Auswahl treffen') or contains(., '\u039F\u03C1\u03AF\u03C3\u03C4\u03B5 \u03C4\u03B9\u03C2 \u03B5\u03C0\u03B9\u03BB\u03BF\u03B3\u03AD\u03C2 \u03C3\u03B1\u03C2') or contains(., 'Set your choices') or contains(., 'Establecer preferencias') or contains(., 'Param\xE9trer vos choix') or contains(., 'V\xE1lassza ki a be\xE1ll\xEDt\xE1sokat') or contains(., 'Imposta le tue scelte') or contains(., 'Stel uw keuzes in') or contains(., 'Ustaw swoje wybory') or contains(., 'Definir suas escolhas') or contains(., 'Seta\u021Bi-v\u0103 op\u021Biunile') or contains(., 'St\xE4ll in dina val')]"] }, { waitForThenClick: ["#sd-cmp", "xpath///span[contains(., '\u041E\u0442\u0445\u0432\u044A\u0440\u043B\u044F\u043C \u0432\u0441\u0438\u0447\u043A\u043E') or contains(., 'Odm\xEDtnout v\u0161e') or contains(., 'Afvis alt') or contains(., 'Alle ablehnen') or contains(., '\u0391\u03C0\u03CC\u03C1\u03C1\u03B9\u03C8\u03B7 \u03CC\u03BB\u03C9\u03BD') or contains(., 'Reject all') or contains(., 'Rechazar todo') or contains(., 'Tout refuser') or contains(., 'Mind elutas\xEDt\xE1sa') or contains(., 'Rifiuta tutto') or contains(., 'Alles weigeren') or contains(., 'Odrzu\u0107 wszystkie') or contains(., 'Rejeitar todos') or contains(., 'Respinge\u021Bi toate') or contains(., 'Avvisa allt')]"] }], else: [{ check: "any", wv: 2105 }, { k: 2105 }] }] }], [], {}], [1, "site-notifications-cookie-usage", 1, "", 22, [2106], [{ e: 2106 }], [{ v: 2106 }], [{ h: 2106 }], [], {}], [1, "sitepark-sitekit", 2, "", 22, [2107], [{ e: 2107 }], [{ v: 2107 }], [{ if: { e: 2108 }, then: [{ timeout: 5e3, optional: true, w: 2109 }, { retry: 5, retryInterval: 500, c: 2110 }], else: [{ h: 2107 }] }], [], {}], [1, "sitevision", 2, "", 22, [2111], [{ e: 2111 }], [{ v: 2111 }], [{ if: { e: 2112 }, then: [{ k: 2112 }], else: [{ k: 2113 }, { wv: 2114 }, { all: true, optional: true, k: 2115 }, { k: 2114 }] }], [{ check: "none", timeout: 2e3, wv: 2111 }], {}], [1, "sitevision-cookie-consent", 2, "", 22, [2116], [{ e: 2116 }], [{ v: 2116 }], [{ if: { e: 2117 }, then: [{ k: 2117 }], else: [{ k: 2118 }, { wv: 2119 }, { all: true, optional: true, k: 2120 }, { k: 2119 }] }], [], {}], [1, "sitevision-iw-cookie", 2, "", 22, [2121], [{ e: 2121 }], [{ v: 2121 }], [{ if: { e: 2122 }, then: [{ k: 2123 }], else: [{ k: 2124 }, { wv: 2125 }, { all: true, optional: true, k: 2126 }, { k: 2127 }] }], [], {}], [1, "smartweb", 0, "", 10, [2128], [{ e: 2129 }], [{ v: 2130 }], [{ all: true, optional: true, k: 2131 }, { c: 2129 }], [{ cc: 2132 }], {}], [1, "snigel", 2, "", 22, [], [{ e: 2133 }], [{ v: 2133 }], [{ k: 2134 }, { k: 2135 }], [{ cc: 2136 }], {}], [1, "socialfunders", 2, "", 22, [2137], [{ e: 2138 }, { e: 2139 }], [{ v: 2137 }], [{ c: 2138 }], [], {}], [1, "sogei-cookiebar", 0, "", 22, [2140], [{ e: 2141 }], [{ v: 2141 }], [{ c: 2142 }], [{ cc: 2143 }], {}], [1, "soleil-sitevision-cookie-message", 2, "", 22, [2144], [{ e: 2145 }], [{ v: 2145 }], [{ c: 2145 }], [{ check: "none", v: 2144 }], {}], [1, "somerset-council", 0, "", 10, [], [{ e: 2146 }], [{ v: 2147 }], [{ c: 2147 }], [{ cc: 2148 }], {}], [1, "soteshop", 2, "", 22, [2149, 2150], [{ e: 2151 }], [{ check: "any", v: 2151 }], [{ if: { e: 2152 }, then: [{ c: 2152 }], else: [{ k: 2153 }] }, { check: "none", wv: 2151 }], [{ if: { e: 2152 }, then: [{ cc: 2154 }, { cc: 2155 }], else: [{ cc: 2156 }] }], {}], [3, "sourcepoint-top", 2, "", 10, [2157], [{ e: 2157 }], [{ v: 2157 }], [{ stylesheet: "body > div[id^='sp_message_container_'] { visibility: hidden !important; z-index: -1 !important; pointer-events: none !important; }", stylesheetId: "sourcepoint-top-hide" }, { stylesheet: "html.sp-message-open { height: auto !important; width: auto !important; } html.sp-message-open body { position: relative !important; overflow: auto !important; margin-top: 0 !important; top: auto !important; left: auto !important; right: auto !important; }", stylesheetId: "sourcepoint-top-scroll-unlock" }, { removeClass: "sp-message-open", selector: "html", optional: true }, { eval: "EVAL_SOURCEPOINT_RESTORE_SCROLL", optional: true }], [], {}], [1, "spaggiari-pvw", 0, "", 10, [2158], [{ e: 2159 }], [{ v: 2159 }], [{ c: 2159 }], [], {}], [1, "sparebank1-optin", 2, "", 22, [2160], [{ e: 2161 }], [{ v: 2160 }], [{ c: 2162 }], [{ check: "none", timeout: 2e3, wv: 2160 }], {}], [1, "squarespace-cookie-banner", 2, "", 22, [2163], [{ e: 2164 }], [{ v: 2165 }], [{ if: { e: 2166 }, then: [{ c: 2166 }], else: [{ c: 2167 }, { wv: 2168 }, { all: true, optional: true, k: 2169 }, { c: 2170 }] }], [{ timeout: 1e3, check: "none", wv: 2165 }], {}], [1, "squarespace-cookie-notice", 1, "", 22, [2163], [{ e: 2171 }, { negated: true, e: 2164 }], [{ v: 2165 }], [{ h: 2163 }], [], {}], [1, "squiz", 0, "", 10, [312], [{ e: 2172 }], [{ v: 2172 }], [{ c: 2173 }], [{ wait: 500 }, { cc: 2174 }], {}], [1, "steampowered.com", 2, "", 22, [], [{ e: 2175 }, { v: 2175 }], [{ v: 2175 }], [{ k: 2176 }], [{ wait: 1e3 }, { eval: "EVAL_STEAMPOWERED_0" }], {}], [1, "stellantis-psa", 2, "", 22, [2177, 2178], [{ e: 2179 }], [{ v: 2179 }], [{ if: { e: 2180 }, then: [{ c: 2180 }], else: [{ if: { v: 2181 }, then: [{ all: true, optional: true, k: 2182 }, { c: 2181 }], else: [{ c: 2183 }] }] }, { check: "none", timeout: 3e3, wv: 2177 }], [{ cc: 2184 }, { cc: 2185 }], {}], [1, "stepstone-ccm", 2, "", 22, [2186], [{ e: 2187 }], [{ v: 2188 }], [{ c: 2189 }, { timeout: 2e3, optional: true, c: 2190 }], [{ cc: 2191 }], {}], [1, "stockholm-stad", 2, "", 22, [2192], [{ e: 2193 }], [{ v: 2192 }], [{ c: 2194 }, { timeout: 2e3, wv: 2195 }, { all: true, optional: true, k: 2196 }, { k: 2197 }], [{ cc: 2198 }], {}], [1, "stright", 2, "", 22, [2199, 2200], [{ e: 2199 }], [{ v: 2199 }], [{ if: { v: 2201 }, then: [{ c: 2201 }], else: [{ c: 2202 }, { wv: 2200 }, { if: { e: 2203 }, then: [{ c: 2203 }], else: [{ all: true, optional: true, k: 2204 }, { c: 2205 }] }] }, { check: "none", wv: 2199 }], [{ check: "none", v: 2199 }], {}], [1, "stripchat.com", 0, "", 22, [2206], [{ v: 2207 }], [{ v: 2206 }], [{ timeout: 5e3, optional: true, w: 2208 }, { c: 2207 }, { timeout: 1500, optional: true, wv: 2209 }, { if: { check: "none", v: 2209 }, then: [{ k: 2207 }] }, { c: 2210 }], [{ wait: 500 }, { cc: 2211 }], {}], [1, "studioibizz-cookieconsent", 2, "", 22, [2212], [{ e: 2212 }], [{ v: 2213 }], [{ if: { e: 2214 }, then: [{ c: 2214 }], else: [{ h: 2215 }] }], [], {}], [1, "substack", 0, "", 12, [], [{ e: 2216 }], [{ v: 2216 }], [{ waitForThenClick: [".pencraft", "xpath///button[contains(., 'Only Necessary') or contains(., 'Reject')]"] }], [{ cc: 2217 }], {}], [1, "summitracing", 1, "", 10, [2218], [{ e: 2219 }], [{ v: 2219 }], [{ h: 2218 }], [], {}], [1, "supercontable", 2, "", 22, [2220], [{ e: 2221 }], [{ v: 2222 }], [{ c: 2223 }, { wv: 2224 }, { if: { e: 2225 }, then: [{ all: true, k: 2225 }] }, { k: 2224 }, { check: "none", wv: 2220 }], [{ wait: 500 }, { any: [{ cc: 2226 }, { cc: 2227 }] }], {}], [1, "swis-avg", 2, "", 22, [2228], [{ e: 2229 }], [{ v: 2230 }], [{ if: { e: 2231 }, then: [{ k: 2231 }, { wv: 2232 }] }, { if: { e: 2233 }, then: [{ k: 2233 }] }, { if: { e: 2234 }, then: [{ k: 2234 }] }, { c: 2235 }], [{ cc: 2236 }, { cc: 2237 }], {}], [1, "swisscom-ecare", 2, "", 22, [2238], [{ e: 2239 }], [{ v: 2240 }], [{ if: { v: 2241 }, then: [{ retry: 3, retryInterval: 500, c: 2241 }], else: [{ k: 2242 }, { wv: 2243 }, { all: true, optional: true, k: 2244 }, { retry: 3, retryInterval: 500, c: 2245 }] }, { check: "none", wv: 2240 }], [{ cc: 2246 }], {}], [1, "synology", 0, "", 22, [2247], [{ e: 2247 }], [{ v: 2247 }], [{ c: 2248 }, { wv: 2249 }, { all: true, optional: true, k: 2250 }, { c: 2251 }], [{ cc: 2252 }, { cc: 2253 }], {}], [1, "tagconcierge", 2, "", 22, [2254], [{ e: 2254 }], [{ v: 2255 }], [{ if: { v: 2256 }, then: [{ k: 2256 }], else: [{ c: 2257 }, { wv: 2258 }, { if: { e: 2259 }, then: [{ k: 2259 }], else: [{ all: true, optional: true, k: 2260 }, { c: 2261 }] }] }], [{ any: [{ check: "none", v: 2254 }, { check: "none", v: 2262 }] }], {}], [1, "takealot.com", 1, "", 22, [2263], [{ e: 2264 }], [{ e: 2264 }], [{ h: 2263 }, { if: { e: 2265 }, then: [{ eval: "EVAL_TAKEALOT_0" }], else: [] }], [], {}], [1, "tallink", 2, "", 22, [2266], [{ e: 2267 }], [{ v: 2266 }], [{ c: 2268 }, { check: "none", timeout: 2e3, optional: true, wv: 2266 }], [{ cc: 2269 }, { negated: true, cc: 2270 }], {}], [1, "tarteaucitron deny", 2, "", 22, [2271], [{ e: 2271 }], [{ v: 2272 }], [{ wait: 500 }, { k: 2272 }], [{ eval: "EVAL_TARTEAUCITRON_2" }], {}], [1, "tarteaucitron.js", 2, "", 22, [2271], [{ e: 2271 }], [{ v: 2273 }, { negated: true, e: 2272 }], [{ if: { e: 2274 }, then: [{ all: true, optional: true, k: 2275 }, { k: 2273 }], else: [{ k: 2276 }, { c: 2277 }, { k: 2278 }] }], [{ eval: "EVAL_TARTEAUCITRON_2" }], {}], [1, "taunton", 2, "", 22, [2279], [{ e: 2279 }], [{ e: 2280 }], [{ optional: true, all: true, k: 2281 }, { k: 2282 }], [{ cc: 2283 }], {}], [1, "tccCmpAlert", 2, "", 22, [2284], [{ e: 2284 }], [{ v: 2284 }], [{ c: 2285 }], [], {}], [1, "Tealium", 2, "", 22, [2286], [{ e: 2287 }, { eval: "EVAL_TEALIUM_0" }], [{ check: "any", v: 2287 }, { eval: "EVAL_TEALIUM_NO_DECISION" }], [{ eval: "EVAL_TEALIUM_1" }, { eval: "EVAL_TEALIUM_DONOTSELL" }, { if: { e: 2288 }, then: [{ c: 2288 }], else: [{ h: 2289 }, { timeout: 1e3, optional: true, c: 2290 }] }], [{ eval: "EVAL_TEALIUM_3" }, { eval: "EVAL_TEALIUM_DONOTSELL_CHECK" }, { check: "none", v: 2291 }], {}], [1, "tempest-cookie-notice", 1, "", 22, [2292], [{ e: 2293 }], [{ v: 2292 }], [{ h: 2292 }], [], {}], [1, "Termly", 2, "", 22, [2294], [{ e: 2294 }], [{ v: 2295 }], [{ if: { e: 2296 }, then: [{ k: 2296 }], else: [{ c: 2297 }, { timeout: 700, w: 2298 }, { if: { e: 2298 }, then: [{ k: 2298 }], else: [{ all: true, c: 2299 }, { c: 2300 }] }] }], [], {}], [1, "termsfeed", 2, "", 22, [2301, 2302, 2303], [{ e: 2304 }], [{ v: 2304 }], [{ if: { e: 2305 }, then: [{ c: 2305 }], else: [{ c: 2306 }, { w: 2307 }, { all: true, optional: true, k: 2308 }, { k: 2309 }] }], [], {}], [1, "termsfeed3", 2, "", 22, [2310], [{ e: 2311 }], [{ v: 2311 }], [{ if: { e: 2312 }, then: [{ k: 2312 }, { wv: 2313 }, { c: 2313 }], else: [{ h: 2310 }] }], [], {}], [1, "Test page CMP", 2, "", 22, [2314], [{ e: 2315 }], [{ v: 2315 }], [{ w: 2314 }, { eval: "EVAL_TESTCMP_STEP" }, { k: 2314 }], [{ eval: "EVAL_TESTCMP_0" }], {}], [1, "Test page cosmetic CMP", 1, "", 22, [2316], [{ e: 2317 }], [{ v: 2317 }], [{ h: 2317 }], [{ wait: 500 }, { eval: "EVAL_TESTCMP_COSMETIC_0" }], {}], [1, "texas-instruments", 2, "", 22, [2318], [{ e: 2319 }], [{ v: 2320 }], [{ k: 2321 }, { wv: 2322 }, { if: { e: 2323 }, then: [{ all: true, k: 2323 }] }, { c: 2324 }], [{ cc: 2325 }], {}], [1, "thalia.de", 2, "", 22, [2326], [{ e: 2327 }], [{ v: 2326 }], [{ k: 2328 }], [], {}], [1, "thefreedictionary.com", 2, "", 22, [2329], [{ e: 2329 }], [{ v: 2329 }], [{ eval: "EVAL_THEFREEDICTIONARY_0" }], [], {}], [1, "tna-cookie-banner", 2, "", 22, [2330], [{ e: 2331 }], [{ v: 2332 }], [{ c: 2332 }, { timeout: 1e3, optional: true, c: 2333 }], [{ cc: 2334 }, { cc: 2335 }], {}], [1, "toyota", 0, "", 10, [2336], [{ e: 2337 }], [{ v: 2338 }], [{ timeout: 5e3, w: 2339 }, { c: 2340 }, { wv: 2341 }, { if: { e: 2342 }, then: [{ timeout: 2e3, optional: true, w: 2343 }] }, { all: true, optional: true, k: 2344 }, { c: 2341 }], [{ cc: 2345 }], {}], [1, "tplink", 0, "", 22, [2346, 2347], [{ e: 2348 }], [{ any: [{ v: 2349 }, { v: 2350 }, { v: 2351 }] }], [{ if: { v: 2350 }, then: [{ k: 2350 }] }, { if: { v: 2352 }, then: [{ k: 2352 }] }, { if: { e: 2353 }, then: [{ wv: 2354 }, { c: 2355 }], else: [{ wv: 2356 }, { if: { e: 2357 }, then: [{ c: 2357 }], else: [{ all: true, optional: true, k: 2358 }, { c: 2359 }] }] }], [{ cc: 2360 }, { negated: true, cc: 2361 }], {}], [1, "tracsis-delay-repay", 0, "", 22, [2362], [{ e: 2363 }], [{ v: 2363 }], [{ retry: 2, c: 2364 }], [{ cc: 2365 }], {}], [1, "trader-joes-com", 1, "", 22, [2366], [{ e: 2366 }], [{ v: 2366 }], [{ h: 2366 }], [], {}], [2, "transcend", 1, "", 22, [2367], [{ e: 2367 }], [{ v: 2367 }], [{ setStyle: "display: none !important; z-index: -1 !important; pointer-events: none !important;", selector: "#transcend-consent-manager" }], [], {}], [1, "travix", 0, "", 10, [], [{ e: 2368 }], [{ v: 2368 }], [{ c: 2369 }], [{ cc: 2370 }], {}], [1, "tri-table", 2, "", 22, [], [{ e: 2371 }], [{ v: 2371 }], [{ retry: 2, retryInterval: 500, c: 2372 }, { if: { e: 2373 }, then: [{ k: 2373 }], else: [{ optional: true, k: 2372 }] }, { check: "none", wv: 2371 }], [{ any: [{ cc: 2374 }, { cc: 2375 }] }], {}], [1, "trillian", 2, "", 22, [2376], [{ e: 2377 }], [{ v: 2376 }], [{ c: 2377 }], [{ cc: 2378 }], {}], [1, "tropicfeel-com", 2, "", 22, [2379], [{ e: 2379 }], [{ check: "any", v: 2380 }], [{ k: 2381 }, { w: 2382 }, { all: true, k: 2383 }, { k: 2384 }], [], {}], [1, "truendo", 2, "", 22, [2385], [{ e: 2385 }], [{ v: 2385 }], [{ if: { e: 2386 }, then: [{ k: 2387 }], else: [{ k: 2388 }] }, { timeout: 2e3, optional: true, wv: 2389 }, { if: { v: 2389 }, then: [{ k: 2389 }] }], [{ cc: 594 }], {}], [1, "truevault-polaris", 0, "", 22, [], [{ exists: ["#polaris-css-lockdown-container", "#polaris-consent-widgets .polaris-consent-widget"] }], [{ visible: ["#polaris-css-lockdown-container", "#polaris-consent-widgets .polaris-consent-widget"] }], [{ if: { exists: ["#polaris-css-lockdown-container", ".polaris-consent-widget__actions button:nth-child(2):not(:last-child)"] }, then: [{ click: ["#polaris-css-lockdown-container", ".polaris-consent-widget__actions button:nth-child(2):not(:last-child)"] }], else: [{ click: ["#polaris-css-lockdown-container", ".polaris-consent-widget__actions button:first-child"] }, { waitFor: ["#polaris-css-lockdown-container", ".polaris-preferences-widget__save_button"], timeout: 2e3 }, { click: ["#polaris-css-lockdown-container", ".polaris-preferences-widget input[type=checkbox]:checked:not([disabled])"], all: true, optional: true }, { click: ["#polaris-css-lockdown-container", ".polaris-preferences-widget input[type=checkbox]:checked:not([disabled])"], all: true, optional: true }, { click: ["#polaris-css-lockdown-container", ".polaris-preferences-widget__save_button"] }] }], [{ waitForVisible: ["#polaris-css-lockdown-container", "#polaris-consent-widgets .polaris-consent-widget"], check: "none", timeout: 2e3 }, { cc: 2390 }], {}], [1, "trust360", 2, "", 22, [2391], [{ e: 2391 }], [{ v: 2391 }], [{ c: 2392 }], [{ cc: 2393 }], {}], [1, "TrustArc-newcm", 2, "", 22, [2394], [{ exists: [".truste_popframe.trustarc_newcm_container", "a.required, a.call, button.required, button.call"] }], [{ visible: [".truste_popframe.trustarc_newcm_container", "a.required, a.call, button.required, button.call"], check: "any" }], [{ waitForThenClick: [".truste_popframe.trustarc_newcm_container", "a.required, button.required"] }], [{ cc: 2395 }, { negated: true, cc: 2396 }], {}], [1, "truyo", 2, "", 22, [2397], [{ e: 2398 }], [{ v: 2397 }], [{ k: 2399 }], [], {}], [1, "trybe", 0, "", 22, [2400], [{ e: 2400 }], [{ v: 2401 }], [{ c: 2402 }, { w: 2403 }, { optional: true, k: 2404 }, { optional: true, k: 2405 }, { c: 2406 }], [{ wait: 500 }, { cc: 2407 }], {}], [1, "tti-cookie-popup", 2, "", 22, [], [{ e: 2408 }], [{ v: 2408 }], [{ if: { v: 2409 }, then: [{ k: 2409 }], else: [{ k: 2408 }, { timeout: 1e4, wv: 2410 }, { if: { v: 2411 }, then: [{ k: 2411 }], else: [{ all: true, optional: true, k: 2412 }, { c: 2413 }] }] }, { timeout: 1e4, check: "none", wv: 2408 }], [{ cc: 2414 }], {}], [1, "tuki-cookieterms", 2, "", 22, [2415], [{ e: 2416 }], [{ v: 2417 }], [{ if: { e: 2418 }, then: [{ k: 2418 }, { wv: 2419 }, { all: true, optional: true, k: 2420 }, { k: 2421 }], else: [{ c: 2422 }] }], [{ cc: 2423 }], {}], [1, "tumblr-custom-domain-gdpr-banner", 1, "", 22, [2424], [{ e: 2424 }], [{ v: 2424 }], [{ h: 2424 }], [{ check: "none", v: 2424 }], {}], [1, "tv2reg-cookie-notice", 2, "", 22, [2425], [{ e: 2426 }, { e: 2427 }], [{ check: "any", v: 2427 }], [{ c: 2427 }, { check: "none", timeout: 2e3, optional: true, wv: 2427 }], [{ cc: 2428 }], {}], [1, "twcc", 0, "", 10, [2429], [{ e: 2430 }], [{ v: 2430 }], [{ c: 2431 }], [{ cc: 2432 }], {}], [1, "txxx-network", 0, "", 22, [], [{ e: 2433 }], [{ v: 2433 }], [{ c: 2433 }], [{ check: "none", v: 2433 }], {}], [1, "typo3-cookieconsent", 2, "", 22, [2434], [{ e: 2435 }], [{ v: 2434 }], [{ c: 2435 }], [{ cc: 2436 }], {}], [1, "tyreseeker", 0, "", 22, [], [{ e: 2437 }], [{ v: 2438 }], [{ c: 2439 }, { wv: 2440 }, { wait: 500 }, { all: true, optional: true, k: 2441 }, { k: 2440 }, { check: "none", wv: 2442 }], [], {}], [1, "u-cookie-consent", 2, "", 22, [2443], [{ e: 2444 }], [{ v: 2444 }], [{ c: 2444 }], [{ check: "none", timeout: 2e3, wv: 2444 }], {}], [1, "u12-data-protection-notice", 2, "", 22, [2445], [{ e: 2445 }], [{ v: 2446 }], [{ c: 2447 }], [], {}], [1, "ubuntu.com", 2, "", 22, [2448], [{ any: [{ e: 2449 }, { e: 2450 }] }], [{ any: [{ v: 2451 }, { v: 2450 }] }], [{ any: [{ c: 2452 }, { c: 2453 }] }, { optional: true, all: true, timeout: 500, c: 2454 }, { any: [{ c: 2455 }, { c: 2456 }] }], [{ cc: 2457 }], {}], [1, "uft-cookie-notice", 2, "", 22, [2458], [{ e: 2459 }], [{ v: 2458 }], [{ k: 2459 }, { c: 2460 }, { check: "none", wv: 2458 }], [{ wait: 1e3 }, { cc: 2461 }], { intermediate: false }], [1, "UK Cookie Consent", 1, "", 22, [2462], [{ e: 2462 }], [{ e: 2463 }], [{ h: 2462 }], [{ negated: true, cc: 2464 }], {}], [1, "uk-parliament", 2, "", 22, [2465, 2466], [{ e: 2467 }], [{ v: 2468 }], [{ c: 2467 }, { optional: true, h: 2466 }], [{ cc: 2469 }], {}], [1, "umcg", 2, "", 22, [2470], [{ e: 2471 }], [{ v: 2472 }], [{ all: true, optional: true, k: 2473 }, { c: 2474 }], [], {}], [1, "uned-barbastro", 2, "", 22, [2475], [{ e: 2475 }, { e: 2476 }], [{ v: 2475 }], [{ c: 2477 }, { wv: 2478 }, { if: { e: 2479 }, then: [{ k: 2479 }] }, { k: 2478 }, { check: "none", timeout: 2e3, optional: true, wv: 2475 }], [{ cc: 2480 }, { negated: true, cc: 2481 }], {}], [1, "unicity", 2, "", 22, [2482], [{ e: 2483 }], [{ v: 2484 }], [{ if: { e: 2485 }, then: [{ k: 2485 }], else: [{ k: 2486 }, { wv: 2487 }, { all: true, optional: true, k: 2488 }, { k: 2489 }] }, { check: "none", timeout: 2e3, optional: true, wv: 2482 }], [{ cc: 2490 }], {}], [1, "united-online", 2, "", 22, [2491, 2492], [{ e: 2493 }], [{ any: [{ check: "any", v: 2494 }, { e: 2495 }] }], [{ if: { check: "any", v: 2496 }, then: [{ c: 2497 }] }, { if: { v: 2498 }, then: [{ h: 2498 }] }, { if: { e: 2495 }, then: [{ k: 2499 }] }], [{ cc: 2500 }], {}], [1, "up-cookie-consent", 2, "", 22, [2501], [{ e: 2501 }], [{ v: 2501 }], [{ if: { e: 2502 }, then: [{ c: 2502 }], else: [{ c: 2503 }, { c: 2504 }] }, { check: "none", timeout: 1e4, wv: 2505 }], [{ cc: 2506 }, { cc: 2507 }], {}], [1, "usercentrics-api", 2, "", 22, [], [{ e: 2508 }], [{ eval: "EVAL_USERCENTRICS_API_0" }, { if: { e: 2509 }, then: [{ timeout: 2e3, wv: 2509 }], else: [{ exists: ["#usercentrics-root", "[data-testid=uc-container]"] }, { timeout: 2e3, wv: 2510 }] }], [{ if: { exists: ["#usercentrics-root", "[data-testid=uc-deny-all-button]"] }, then: [{ click: ["#usercentrics-root", "[data-testid=uc-deny-all-button]"] }], else: [{ if: { exists: ["#usercentrics-cmp-ui", "[data-action-type=deny]"] }, then: [{ click: ["#usercentrics-cmp-ui", "[data-action-type=deny]"] }], else: [{ eval: "EVAL_USERCENTRICS_API_1" }, { eval: "EVAL_USERCENTRICS_API_2" }] }] }, { removeClass: "overflowHidden", selector: "body", optional: true }], [{ eval: "EVAL_USERCENTRICS_API_6" }], {}], [1, "usercentrics-button", 2, "", 22, [], [{ e: 2511 }], [{ v: 2512 }], [{ k: 2513 }], [{ eval: "EVAL_USERCENTRICS_BUTTON_0" }], {}], [1, "uw-madison-cookieconsent", 1, "", 22, [2514], [{ e: 2515 }], [{ v: 2514 }], [{ h: 2516 }], [], {}], [1, "vdg-cookies", 2, "", 22, [2517], [{ e: 2517 }], [{ v: 2517 }], [{ c: 2518 }, { c: 2519 }, { check: "none", wv: 2517 }], [{ cc: 2520 }], {}], [1, "vendic-cookiebar", 0, "", 22, [2521, 2522], [{ e: 2523 }], [{ v: 2524 }], [{ c: 2523 }], [{ check: "none", timeout: 3e3, wv: 2524 }], {}], [1, "vergelijkgroep", 0, "", 10, [2525], [{ e: 2526 }], [{ v: 2527 }], [{ c: 2528 }], [{ cc: 2529 }], {}], [1, "vivenu", 2, "", 22, [], [{ e: 2530 }], [{ v: 2530 }], [{ c: 2531 }], [{ check: "none", v: 2530 }], {}], [1, "vortex-consent", 2, "", 22, [2532, 2533], [{ e: 2534 }], [{ v: 2534 }], [{ c: 2534 }], [{ cc: 2535 }, { negated: true, cc: 2536 }], {}], [1, "vrg-bottom-cookies", 2, "", 22, [2537], [{ e: 2538 }], [{ v: 2537 }], [{ c: 2539 }, { timeout: 1e4, wv: 2540 }, { if: { e: 2541 }, then: [{ all: true, k: 2541 }] }, { retry: 2, retryInterval: 1e3, c: 2540 }], [{ check: "none", timeout: 5e3, wv: 2537 }], {}], [1, "vue-cookie-accept-decline", 0, "", 10, [2542, 2543], [{ e: 2544 }], [{ v: 2545 }], [{ if: { e: 2546 }, then: [{ c: 2546 }], else: [{ h: 2545 }] }], [], {}], [1, "vue-cookie-law", 0, "", 10, [2547], [{ e: 2548 }], [{ v: 2547 }], [{ if: { e: 2549 }, then: [{ c: 2549 }], else: [{ h: 2547 }] }], [], {}], [1, "waitrose.com", 2, "", 22, [2550, 2551, 2552], [{ e: 2551 }], [{ v: 2551 }], [{ k: 2553 }, { wait: 200 }, { eval: "EVAL_WAITROSE_0" }, { k: 2554 }], [{ cc: 2555 }, { cc: 2556 }], {}], [1, "warning-cookie-notice", 1, "", 22, [2557], [{ e: 2558 }], [{ v: 2557 }], [{ h: 2557 }], [], {}], [1, "watchfinder", 0, "", 22, [2559], [{ e: 2560 }, { e: 2561 }], [{ v: 2560 }], [{ c: 2560 }, { wv: 2561 }, { all: true, optional: true, k: 2562 }, { c: 2563 }], [{ cc: 2564 }, { cc: 2565 }], {}], [2, "wbd-legal-terms", 1, "", 22, [], [{ e: 2566 }, { negated: true, e: 2567 }], [{ check: "any", v: 2568 }], [{ if: { e: 2569 }, then: [{ eval: "EVAL_WBD_LTP_CLOSE" }, { removeClass: "is-NoScroll", selector: "body", optional: true }], else: [{ h: 2570 }, { addStyle: "overflow: auto !important", selector: "body", optional: true }] }], [], {}], [1, "webflow", 2, "", 22, [2571], [{ any: [{ v: 2571 }, { e: 2572 }] }], [{ v: 2571 }, { v: 2573 }], [{ if: { e: 2574 }, then: [{ wait: 500 }, { c: 2574 }], else: [{ h: 2575 }] }], [{ cc: 2576 }], {}], [1, "weblegal", 2, "", 22, [2577, 2578], [{ e: 2579 }], [{ v: 2577 }, { e: 2580 }], [{ c: 2579 }], [{ cc: 2581 }], {}], [1, "weblication", 2, "", 22, [2582], [{ e: 2583 }], [{ v: 2582 }], [{ if: { e: 2584 }, then: [{ k: 2585 }] }, { if: { e: 2586 }, then: [{ k: 2587 }] }, { c: 2588 }], [{ cc: 2589 }], {}], [1, "websale", 2, "", 22, [], [{ e: 2590 }, { e: 2591 }], [{ check: "any", v: 2592 }], [{ all: true, optional: true, k: 2593 }, { all: true, optional: true, k: 2594 }, { if: { e: 2595 }, then: [{ retry: 3, retryInterval: 500, c: 2595 }], else: [{ c: 2596 }] }], [{ cc: 2597 }], {}], [1, "webtoffee-cookie-consent", 2, "", 22, [2598], [{ e: 2599 }], [{ v: 2599 }], [{ if: { e: 2600 }, then: [{ c: 2600 }], else: [{ if: { e: 2601 }, then: [{ c: 2601 }, { wv: 2602 }, { optional: true, k: 2603 }, { c: 2604 }], else: [{ c: 2605 }, { wv: 2606 }, { all: true, optional: true, k: 2607 }, { c: 2606 }] }] }], [{ cc: 2608 }], {}], [1, "whitepages-gpc-banner", 2, "", 22, [2609], [{ e: 2610 }], [{ v: 2609 }], [{ c: 2610 }], [{ check: "none", v: 2609 }], {}], [1, "wiki.gg", 1, "", 22, [2611], [{ v: 2611 }], [{ v: 2611 }], [{ h: 2611 }], [], {}], [1, "wimc-gtm-consent", 2, "", 22, [2612], [{ e: 2612 }], [{ v: 2613 }], [{ if: { e: 2614 }, then: [{ c: 2614 }], else: [{ c: 2615 }, { timeout: 2e3, wv: 2616 }, { all: true, optional: true, k: 2617 }, { c: 2618 }] }, { check: "none", timeout: 2e3, wv: 2613 }], [{ cc: 2619 }, { cc: 2620 }], {}], [1, "wirtualna-polska", 2, "", 22, [2621], [{ exists: ["div[role=dialog][aria-modal=true][aria-label*='WP']", "xpath/.//button[normalize-space(.)='Ustawienia zaawansowane' or normalize-space(.)='Advanced settings' or normalize-space(.)='Erweiterte einstellungen' or normalize-space(.)='Pokro\u010Dil\xE1 nastaven\xED' or normalize-space(.)='Roz\u0161\xEDren\xE9 nastavenia' or normalize-space(.)='I\u0161pl\u0117stiniai nustatymai']"] }], [{ v: 2621 }], [{ waitForThenClick: ["div[role=dialog][aria-modal=true][aria-label*='WP']", "xpath/.//button[normalize-space(.)='Ustawienia zaawansowane' or normalize-space(.)='Advanced settings' or normalize-space(.)='Erweiterte einstellungen' or normalize-space(.)='Pokro\u010Dil\xE1 nastaven\xED' or normalize-space(.)='Roz\u0161\xEDren\xE9 nastavenia' or normalize-space(.)='I\u0161pl\u0117stiniai nustatymai']"] }, { waitForThenClick: ["div[role=dialog][aria-modal=true][aria-label*='WP']", "xpath/.//button[normalize-space(.)='Zapisz' or normalize-space(.)='Save' or normalize-space(.)='Speichern' or normalize-space(.)='Ulo\u017Eit' or normalize-space(.)='Ulo\u017Ei\u0165' or normalize-space(.)='I\u0161saugoti']"] }, { check: "none", timeout: 5e3, optional: true, wv: 2621 }], [], {}], [1, "wix", 0, "", 22, [], [{ e: 2622 }], [{ v: 2622 }], [{ if: { e: 2623 }, then: [{ wait: 1e3 }, { if: { e: 2624 }, then: [{ k: 2624 }], else: [{ k: 2625 }, { wv: 2626 }, { all: true, optional: true, k: 2627 }, { k: 2626 }] }, { any: [{ check: "none", timeout: 2e3, wv: 2623 }, { h: 2623 }] }], else: [{ if: { e: 2628 }, then: [{ k: 2628 }], else: [{ h: 2629 }] }] }], [], {}], [1, "woo-commerce-com", 2, "", 22, [2630], [{ e: 2630 }], [{ v: 2630 }], [{ if: { e: 2631 }, then: [{ c: 2632 }], else: [{ c: 2633 }, { wv: 2634 }, { all: true, optional: true, k: 2635 }, { c: 2636 }] }], [{ cc: 2637 }], {}], [1, "workday", 2, "", 22, [2638], [{ e: 2639 }], [{ v: 2638 }], [{ c: 2639 }], [{ cc: 2640 }], {}], [1, "worldbank", 2, "", 22, [2641, 2642], [{ e: 2643 }], [{ v: 2644 }], [{ c: 2645 }], [{ cc: 2646 }], {}], [1, "WP Cookie Notice for GDPR", 2, "", 22, [2647], [{ v: 2647 }], [{ v: 2647 }], [{ c: 2648 }], [{ cc: 2649 }], {}], [1, "WP DSGVO Tools", 2, "", 22, [2650], [{ e: 2651 }], [{ check: "any", v: 2651 }], [{ if: { e: 2652 }, then: [{ c: 2652 }], else: [{ c: 2653 }] }], [{ cc: 2654 }], {}], [1, "wpcc", 1, "", 22, [2655], [{ e: 2655 }], [{ e: 2656 }], [{ h: 2655 }], [], {}], [1, "WPConsent", 2, "", 22, [2657], [{ e: 2658 }], [{ visible: ["#wpconsent-container", "#wpconsent-banner-holder.wpconsent-banner-visible"] }], [{ waitForThenClick: ["#wpconsent-container", "#wpconsent-banner-holder .wpconsent-cancel-all"] }], [{ cc: 2659 }], {}], [1, "wuerth", 2, "", 22, [2660, 2661], [{ e: 2662 }, { e: 2663 }], [{ v: 2662 }], [{ if: { v: 2664 }, then: [{ k: 2664 }], else: [{ c: 2663 }, { timeout: 2e3, wv: 2665 }, { all: true, optional: true, k: 2666 }, { k: 2665 }] }], [{ cc: 1734 }], {}], [1, "x28-privacy-settings", 0, "", 22, [], [{ e: 2667 }], [{ v: 2667 }], [{ all: true, optional: true, k: 2668 }, { retry: 3, retryInterval: 1e3, c: 2667 }], [{ cc: 2669 }], {}], [1, "xenforo-cookie-consent", 2, "", 22, [2670], [{ e: 2671 }], [{ v: 2672 }], [{ if: { e: 2673 }, then: [{ k: 2673 }], else: [{ timeout: 2e3, retry: 10, retryInterval: 500, c: 2674 }, { timeout: 2e3, wv: 2675 }, { all: true, optional: true, k: 2676 }, { k: 2675 }] }], [{ timeout: 2e3, check: "none", wv: 2670 }, { cc: 2677 }], {}], [1, "xgroovy", 1, "", 22, [2678], [{ e: 2679 }], [{ v: 2679 }], [{ h: 2678 }], [], {}], [1, "xing.com", 2, "", 22, [], [{ e: 2680 }], [{ e: 2680 }], [{ k: 2681 }, { k: 2682 }], [{ cc: 2683 }], {}], [1, "xnxx-com", 1, "", 22, [2684], [{ any: [{ e: 2684 }, { e: 2685 }] }], [{ any: [{ v: 2684 }, { v: 2686 }] }], [{ if: { v: 2686 }, then: [{ c: 2687 }], else: [{ h: 2684 }] }], [], {}], [1, "yle-consent-sdk", 2, "", 22, [2688], [{ e: 2689 }], [{ v: 2688 }], [{ c: 2689 }, { timeout: 2e3, check: "none", wv: 2688 }], [], {}], [1, "youtube-desktop", 2, "", 22, [2690, 2691], [{ e: 2692 }, { e: 2693 }], [{ v: 2692 }], [{ c: 2694 }, { wait: 500 }], [{ wait: 500 }, { cc: 1237 }], {}], [1, "youtube-mobile", 2, "", 22, [2695], [{ e: 2696 }], [{ v: 2696 }], [{ wait: 500 }, { c: 2697 }, { wait: 500 }], [{ wait: 500 }, { cc: 1237 }], {}], [1, "zdf", 2, "", 22, [2698], [{ e: 2698 }], [{ v: 2699 }], [{ c: 2700 }], [], {}], [1, "zoho", 0, "", 22, [2701], [{ e: 2702 }], [{ v: 2701 }], [{ if: { v: 2703 }, then: [{ k: 2703 }], else: [{ k: 2704 }, { wv: 2705 }, { all: true, optional: true, k: 2706 }, { k: 2705 }] }, { check: "none", timeout: 2e3, wv: 2701 }], [{ eval: "EVAL_ZOHO_TEST" }], {}], [1, "zorginstituut-cookiewarning", 0, "", 22, [2707], [{ e: 2708 }], [{ v: 2708 }], [{ c: 2708 }], [{ check: "none", timeout: 2e3, wv: 2709 }], {}], [1, "zurich-cookies-notification-banner", 0, "", 10, [2710], [{ e: 2711 }], [{ v: 2712 }], [{ c: 2711 }], [{ cc: 2713 }], {}], [1, "cookiealert", 2, "", 11, [], [{ e: 2714 }], [{ v: 2715 }], [{ k: 2716 }, { all: true, optional: true, k: 2717 }, { k: 2718 }, { eval: "EVAL_COOKIEALERT_0" }], [{ eval: "EVAL_COOKIEALERT_2" }], { intermediate: false }], [1, "piano.io", 2, "", 1, [], [{ e: 2719 }], [{ v: 2719 }], [{ c: 2720 }], [], {}], [1, "auto_AU_help.dropbox.com_4ad_+1", 0, "^https?://(www\\.)?dropbox\\.com/|^https?://(www\\.)?dropbox\\.com/", 1, [], [{ e: 2721 }], [{ v: 2721 }], [{ wait: 500 }, { c: 2721 }], [{ timeout: 1e3, check: "none", wv: 2721 }], {}], [1, "auto_CA_coasthotels.com_f8m", 0, "^https?://(www\\.)?coasthotels\\.com/", 1, [], [{ e: 2722 }], [{ v: 2722 }], [{ c: 2722 }], [], {}], [1, "auto_CA_epihunter.eu_hd4", 0, "^https?://(www\\.)?combell\\.com/", 1, [], [{ e: 2723 }], [{ v: 2723 }], [{ wait: 500 }, { c: 2723 }], [{ timeout: 1e3, check: "none", wv: 2723 }], {}], [1, "auto_CA_nationalarchives.gov.uk_rx0", 0, "^https?://(www\\.)?webarchive\\.nationalarchives\\.gov\\.uk/", 1, [], [{ e: 2724 }], [{ v: 2724 }], [{ wait: 500 }, { c: 2724 }], [{ timeout: 1e3, check: "none", wv: 2724 }], {}], [1, "auto_CA_natureconservancy.ca_3pp", 0, "^https?://(www\\.)?natureconservancy\\.ca/", 1, [], [{ e: 2725 }], [{ v: 2725 }], [{ wait: 500 }, { c: 2725 }], [{ timeout: 1e3, check: "none", wv: 2725 }], {}], [1, "auto_CA_ontariospca.ca_k32", 0, "^https?://(www\\.)?widget-next\\.clym-sdk\\.net/", 1, [], [{ e: 2726 }], [{ v: 2726 }], [{ wait: 500 }, { c: 2726 }], [{ timeout: 1e3, check: "none", wv: 2726 }], {}], [1, "auto_CA_phoenixnap.com_hrb", 0, "^https?://(www\\.)?widget-next\\.clym-sdk\\.net/", 1, [], [{ e: 2727 }], [{ v: 2727 }], [{ wait: 500 }, { c: 2727 }], [{ timeout: 1e3, check: "none", wv: 2727 }], {}], [1, "auto_CH_phoenixnap.com_lx5", 0, "^https?://(www\\.)?widget-next\\.clym-sdk\\.net/", 1, [], [{ e: 2728 }], [{ v: 2728 }], [{ wait: 500 }, { c: 2728 }], [{ timeout: 1e3, check: "none", wv: 2728 }], {}], [1, "auto_CH_swisscare.com_arx", 0, "^https?://(www\\.)?swisscare\\.com/", 1, [], [{ e: 2729 }], [{ v: 2729 }], [{ wait: 500 }, { c: 2729 }], [{ timeout: 1e3, check: "none", wv: 2729 }], {}], [1, "auto_DE_alfaromeo.de_gvg_+2", 0, "^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/|^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/|^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/", 1, [], [{ e: 2730 }], [{ v: 2730 }], [{ wait: 500 }, { c: 2730 }], [{ timeout: 1e3, check: "none", wv: 2730 }], {}], [1, "auto_DE_huss-licht-ton.de_jj0", 0, "^https?://(www\\.)?huss-licht-ton\\.de/", 1, [], [{ e: 2731 }], [{ v: 2731 }], [{ wait: 500 }, { c: 2731 }], [{ timeout: 1e3, check: "none", wv: 2731 }], {}], [1, "auto_DE_modellbau-berlinski.de_8vm", 0, "^https?://(www\\.)?modellbau-berlinski\\.de/", 1, [], [{ e: 2732 }], [{ v: 2732 }], [{ wait: 500 }, { c: 2732 }], [{ timeout: 1e3, check: "none", wv: 2732 }], {}], [1, "auto_DE_phoenixnap.com_xq7", 0, "^https?://(www\\.)?widget-next\\.clym-sdk\\.net/", 1, [], [{ e: 2733 }], [{ v: 2733 }], [{ wait: 500 }, { c: 2733 }], [{ timeout: 1e3, check: "none", wv: 2733 }], {}], [1, "auto_FR_fiat.fr_3fh", 0, "^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/", 1, [], [{ e: 2734 }], [{ v: 2734 }], [{ wait: 500 }, { c: 2734 }], [{ timeout: 1e3, check: "none", wv: 2734 }], {}], [1, "auto_FR_stellantis.com_67q", 0, "^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/", 1, [], [{ e: 2734 }], [{ v: 2734 }], [{ wait: 500 }, { c: 2734 }], [{ timeout: 1e3, check: "none", wv: 2734 }], {}], [1, "auto_GB_dropbox.com_0_+1", 0, "^https?://(www\\.)?dropbox\\.com/|^https?://(www\\.)?dropbox\\.com/", 1, [], [{ e: 2735 }], [{ v: 2735 }], [{ c: 2735 }], [], {}], [1, "auto_GB_investing.thisismoney.co.uk_0", 0, "^https?://(www\\.)?thisismoney\\.co\\.uk/", 1, [], [{ e: 2736 }], [{ v: 2736 }], [{ c: 2736 }], [], {}], [1, "auto_GB_village-hotels.co.uk_hsa", 0, "^https?://(www\\.)?village-hotels\\.co\\.uk/", 1, [], [{ e: 2737 }], [{ v: 2737 }], [{ wait: 500 }, { c: 2737 }], [{ timeout: 1e3, check: "none", wv: 2737 }], {}], [1, "auto_NL_fiat.nl_trv_+1", 0, "^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/|^https?://(www\\.)?cookielaw\\.emea\\.fcagroup\\.com/", 1, [], [{ e: 2734 }], [{ v: 2734 }], [{ wait: 500 }, { c: 2734 }], [{ timeout: 1e3, check: "none", wv: 2734 }], {}], [1, "auto_NL_flitsmeister.nl_ws8", 0, "^https?://(www\\.)?cookies\\.flitsmeister\\.com/", 1, [], [{ e: 2738 }], [{ v: 2738 }], [{ wait: 500 }, { c: 2738 }], [{ timeout: 1e3, check: "none", wv: 2738 }], {}], [1, "auto_NL_interpolis.nl_jk9", 0, "^https?://(www\\.)?interpolis\\.nl/", 1, [], [{ e: 2739 }], [{ v: 2739 }], [{ wait: 500 }, { c: 2739 }], [{ timeout: 1e3, check: "none", wv: 2739 }], {}], [1, "auto_US_dropbox.com_0_+1", 0, "^https?://(www\\.)?dropbox\\.com/|^https?://(www\\.)?dropbox\\.com/", 1, [], [{ e: 2740 }], [{ v: 2740 }], [{ text: "Decline", c: 2740 }], [], {}], [1, "bmedonline.es", 0, "^https?://(www\\.)?bmedonline\\.es/", 11, [2741], [{ e: 2742 }], [{ v: 2741 }], [{ c: 2743 }], [{ wait: 500 }, { cc: 2744 }], {}], [1, "coinbase", 2, "^https://(www|help)\\.coinbase\\.com", 11, [], [{ e: 2745 }], [{ v: 2745 }], [{ k: 2746 }, { all: true, optional: true, k: 2747 }, { k: 2748 }], [{ eval: "EVAL_COINBASE_0" }], { intermediate: false }], [1, "ecoprivacy", 2, "^https://ecoprivacy\\.ecocerved\\.it/", 1, [], [{ e: 2749 }], [{ v: 2749 }], [{ wv: 2750 }, { wait: 500 }, { k: 2749 }], [], {}], [1, "figma", 2, "^https://([a-z0-9-]+\\.)*figma\\.com/", 11, [2751, 2752], [{ e: 2753 }], [{ v: 2754 }], [{ c: 2755 }], [{ cc: 2756 }, { cc: 2757 }], {}], [1, "gmx-permission", 2, "^https://plus\\.gmx\\.com/lt(?:[?#]|$)", 1, [2758], [{ e: 2758 }], [{ v: 2759 }], [{ if: { e: 2760 }, then: [{ c: 2760 }], else: [{ if: { e: 2761 }, then: [{ c: 2761 }, { wv: 2762 }, { c: 2762 }], else: [{ c: 2762 }] }] }], [{ cc: 2763 }], {}], [1, "nojazz", 2, "^https://nojazz\\.eu/", 1, [], [{ e: 2764 }], [{ v: 2764 }], [{ if: { v: 2765 }, then: [{ k: 2765 }], else: [{ k: 2764 }, { w: 2766 }, { all: true, optional: true, k: 2767 }, { c: 2768 }] }], [], {}], [1, "pdf24", 2, "^https://consent\\.pdf24\\.org/", 1, [], [{ e: 2769 }], [{ v: 2770 }], [{ if: { v: 2771 }, then: [{ k: 2771 }], else: [{ k: 2769 }, { wv: 2772 }, { c: 2773 }] }], [], {}], [1, "privacymanager.io", 2, "^https://cmp-consent-tool\\.privacymanager\\.io/", 1, [2774, 2775], [{ e: 2776 }], [{ v: 2776 }], [{ if: { e: 2777 }, then: [{ k: 2777 }, { c: 2778 }], else: [{ c: 2779 }, { w: 2780 }, { w: 2781 }, { all: true, optional: true, k: 2782 }, { k: 2781 }] }], [], {}], [1, "tumblr-com", 2, "^https://(www\\.)?tumblr\\.com/", 21, [2783], [{ e: 2783 }], [{ check: "any", v: 2783 }], [{ waitForThenClick: ["#cmp-app-container iframe", ".cmp-components-button.is-secondary"], timeout: 5e3 }], [], {}], [1, "web.de", 2, "^https://([a-z]*\\.)?web\\.de/|^https://([a-z]*\\.)?gmx\\.net/", 1, [], [{ e: 2784 }, { e: 2785 }], [{ v: 2785 }], [{ c: 2785 }], [], {}], [1, "widiba", 0, "^https?://(www\\.)?widiba\\.it/", 11, [2786], [{ e: 2787 }], [{ v: 2786 }], [{ c: 2788 }], [{ check: "none", timeout: 2e3, wv: 2786 }], {}], [1, "1pmobile.com", 2, "^https?://(\\w+\\.)?1pmobile\\.com/", 22, [2789], [{ e: 2790 }], [{ v: 2791 }], [{ if: { e: 2792 }, then: [{ all: true, optional: true, k: 2793 }, { c: 2794 }], else: [{ c: 2795 }] }], [], {}], [1, "aa-cookie-banner", 2, "^https://(www\\.)?(envoyair|psaairlines)\\.com/", 22, [2796], [{ exists: ["adc-cookie-banner", "#cookie-banner"] }], [{ visible: ["adc-cookie-banner", "#cookie-banner"] }], [{ waitForThenClick: ["adc-cookie-banner", "#toast-dismiss-button"] }], [{ cc: 2797 }], {}], [1, "abc", 2, "^https://([a-z0-9-]+\\.)?abc\\.net\\.au/", 22, [], [{ e: 2798 }], [{ v: 2799 }], [{ c: 2800 }], [{ cc: 2801 }], {}], [1, "abcacupuncture1", 2, "^https?://(www\\.)?abcacupuncture1\\.com/", 22, [832], [{ any: [{ e: 833 }, { e: 1169 }] }], [{ any: [{ v: 833 }, { v: 2802 }] }], [{ if: { v: 834 }, then: [{ k: 834 }] }, { timeout: 4e3, optional: true, wv: 2802 }, { if: { v: 2802 }, then: [{ k: 1171 }, { all: true, optional: true, k: 1173 }, { optional: true, k: 1174 }] }, { if: { v: 834 }, then: [{ c: 834 }] }], [{ cc: 846 }], {}], [1, "abetterrouteplanner", 2, "^https?://(www\\.)?abetterrouteplanner\\.com/", 22, [], [{ e: 2803 }], [{ v: 2803 }], [{ c: 2803 }], [{ negated: true, e: 2803 }], {}], [1, "accurx", 2, "^https://(accurx\\.nhs\\.uk|patient\\.accurx\\.com)/", 22, [2804], [{ e: 2805 }], [{ v: 2805 }], [{ c: 2806 }], [{ check: "none", timeout: 2e3, wv: 2805 }], {}], [1, "aceodds.com", 2, "^https?://(www\\.)?aceodds\\.com/", 22, [312], [{ e: 2807 }], [{ v: 312 }], [{ retry: 10, retryInterval: 500, c: 2808 }, { check: "none", wv: 312 }], [{ cc: 2809 }], {}], [1, "activobank.pt", 2, "^https://(www\\.)?activobank\\.pt", 22, [2810], [{ e: 2811 }], [{ v: 2812 }], [{ c: 2813 }], [], {}], [1, "adblockplus", 0, "^https?://(\\w+\\.)?adblockplus\\.org/", 10, [892, 2814], [{ e: 2815 }], [{ check: "any", v: 2815 }], [{ if: { v: 2816 }, then: [{ c: 2816 }, { wv: 2817 }, { if: { e: 2818 }, then: [{ k: 2819 }] }, { k: 2817 }], else: [{ c: 2820 }, { wv: 2821 }, { if: { e: 2822 }, then: [{ k: 2823 }] }, { k: 2821 }] }], [{ cc: 2824 }], {}], [1, "adeac-jp", 1, "^https?://(www\\.)?adeac\\.jp/", 22, [2825], [{ e: 2826 }], [{ v: 2825 }], [{ h: 2825 }], [], {}], [1, "adultfriendfinder", 2, "^https://(www\\.)?adultfriendfinder\\.com/", 22, [2827], [{ e: 2828 }], [{ v: 2828 }], [{ c: 2829 }], [{ eval: "EVAL_ADULTFRIENDFINDER_TEST" }], {}], [1, "aemet", 1, "^https?://(www\\.)?aemet\\.es/", 22, [2830], [{ e: 2831 }], [{ v: 2830 }], [{ h: 2830 }], [{ check: "none", v: 2830 }], {}], [1, "aeolservice", 2, "^https?://cloud\\.aeolservice\\.es/", 22, [], [{ e: 2832 }], [{ v: 2833 }], [{ c: 2833 }, { wv: 2834 }, { all: true, optional: true, k: 2835 }, { c: 2836 }], [], {}], [1, "agenziaentrate-geoportale", 1, "^https?://geoportale\\.cartografia\\.agenziaentrate\\.gov\\.it/", 22, [892], [{ e: 2837 }], [{ v: 892 }], [{ h: 892 }], [{ check: "none", v: 892 }], {}], [1, "ah.nl", 0, "^https?://(www\\.)?ah\\.nl/", 10, [], [{ e: 2838 }], [{ v: 2838 }], [{ c: 2839 }], [], {}], [1, "airbnb", 2, "^https?://(www\\.)?airbnb\\.[a-z.]+/", 22, [2840], [{ e: 2841 }], [{ v: 2841 }], [{ retry: 5, retryInterval: 500, c: 2841 }], [{ timeout: 1e3, check: "none", wv: 2841 }], {}], [1, "alaskaair", 1, "^https://(www\\.)?alaskaair\\.com/", 22, [2842], [{ e: 2842 }], [{ v: 2842 }], [{ h: 2842 }], [], {}], [1, "aliasdmc", 2, "^https?://([\\w-]+\\.)?aliasdmc\\.fr/", 22, [2843, 2844], [{ e: 2843 }], [{ v: 2843 }], [{ c: 2845 }, { wv: 2846 }, { all: true, k: 2846 }, { c: 2847 }, { check: "none", wv: 2843 }], [{ cc: 2848 }], {}], [1, "aliexpress", 2, "^https://([a-z]*\\.)?aliexpress\\.com/", 22, [2849], [{ e: 2850 }], [{ check: "any", v: 2850 }], [{ if: { e: 2851 }, then: [{ c: 2852 }], else: [{ if: { e: 2853 }, then: [{ if: { e: 2854 }, then: [{ c: 2855 }], else: [{ c: 2856 }, { timeout: 5e3, c: 2857 }] }], else: [{ if: { e: 2858 }, then: [{ c: 2859 }], else: [{ c: 2860 }, { timeout: 5e3, wv: 2861 }, { c: 2862 }] }] }] }], [], {}], [1, "allabout-japan", 2, "^https?://(\\w+\\.)?allabout-japan\\.com/", 22, [2863], [{ e: 2864 }], [{ v: 2864 }], [{ h: 2863 }, { timeout: 8e3, optional: true, wv: 2802 }, { if: { v: 2802 }, then: [{ k: 1171 }, { all: true, optional: true, k: 1173 }, { optional: true, k: 1174 }] }], [], {}], [1, "ally", 1, "^https://(www\\.)?ally\\.com/", 22, [2865], [{ e: 2866 }], [{ v: 2866 }], [{ h: 2865 }], [], {}], [1, "amazon-music", 2, "^https?://music\\.amazon\\.[a-z.]+/", 22, [2867], [{ e: 2868 }], [{ v: 2869 }], [{ waitForThenClick: ["#bannerFooter music-button[variant=outline]", "button"] }], [{ check: "none", timeout: 2e3, wv: 2869 }], {}], [1, "amazon-pay", 2, "^https?://pay\\.amazon\\.", 22, [2870], [{ e: 2871 }], [{ v: 2870 }], [{ k: 2871 }], [{ cc: 2872 }], {}], [1, "amazonforum", 2, "^https?://(\\w+\\.)?amazonforum\\.com/", 10, [], [{ exists: ["c-cookie-consent", ".cookiecon-footer-container-fixed button.cookie_reject_btn"] }], [{ visible: ["c-cookie-consent", ".cookiecon-footer-container-fixed"] }], [{ waitForThenClick: ["c-cookie-consent", ".cookiecon-footer-container-fixed button.cookie_reject_btn"] }, { waitFor: ["c-cookie-consent", "section.azCookieCustomise li lightning-input", "lightning-primitive-input-toggle", "input"] }, { if: { exists: ["c-cookie-consent", "section.azCookieCustomise li:nth-child(1) lightning-input", "lightning-primitive-input-toggle", "input:checked"] }, then: [{ click: ["c-cookie-consent", "section.azCookieCustomise li:nth-child(1) lightning-input", "lightning-primitive-input-toggle", "input:checked"] }] }, { if: { exists: ["c-cookie-consent", "section.azCookieCustomise li:nth-child(2) lightning-input", "lightning-primitive-input-toggle", "input:checked"] }, then: [{ click: ["c-cookie-consent", "section.azCookieCustomise li:nth-child(2) lightning-input", "lightning-primitive-input-toggle", "input:checked"] }] }, { if: { exists: ["c-cookie-consent", "section.azCookieCustomise li:nth-child(3) lightning-input", "lightning-primitive-input-toggle", "input:checked"] }, then: [{ click: ["c-cookie-consent", "section.azCookieCustomise li:nth-child(3) lightning-input", "lightning-primitive-input-toggle", "input:checked"] }] }, { click: ["c-cookie-consent", "section.azCookieCustomise footer .cookiecon-desktop-section lightning-button.focusStyleButton", "button"] }], [{ waitForVisible: ["c-cookie-consent", ".cookiecon-footer-container-fixed"], timeout: 2e3, check: "none" }], {}], [1, "animatetimes", 2, "^https?://(www\\.)?animatetimes\\.com/", 22, [2873], [{ e: 2873 }], [{ v: 2873 }], [{ c: 2874 }], [{ check: "none", v: 2873 }], {}], [1, "annunci69.it", 2, "^https?://(www\\.)?annunci69\\.it/", 22, [2875], [{ e: 2876 }], [{ v: 2875 }], [{ c: 2877 }, { wv: 2878 }, { optional: true, k: 2879 }, { k: 2880 }, { check: "none", wv: 2875 }], [{ cc: 2881 }], {}], [1, "app.discuss.io", 2, "^https?://app\\.discuss\\.io/", 10, [], [{ e: 1903 }], [{ v: 1903 }], [{ c: 1903 }], [{ cc: 272 }], {}], [1, "arbeidsplassen-nav-no", 2, "^https?://arbeidsplassen\\.nav\\.no/", 22, [2882], [{ e: 2883 }], [{ v: 2882 }], [{ c: 2884 }], [{ cc: 858 }], {}], [1, "arena.ai", 2, "^https?://(\\w+\\.)?arena\\.ai/", 22, [], [{ e: 2885 }], [{ v: 2885 }], [{ c: 2886 }, { wv: 2887 }, { all: true, optional: true, k: 2888 }, { c: 2889 }], [{ cc: 2890 }], {}], [1, "aslroma1-it", 2, "^https?://(\\w+\\.)?aslroma1\\.it/", 22, [2891], [{ e: 2892 }], [{ v: 2891 }], [{ c: 2892 }, { check: "none", wv: 2891 }], [{ cc: 2893 }], {}], [1, "asrock", 2, "^https?://(\\w+\\.)?asrock\\.com/", 22, [], [{ e: 2894 }], [{ v: 2895 }], [{ c: 2896 }, { wv: 2897 }, { all: true, optional: true, k: 2898 }, { k: 2899 }], [{ cc: 2900 }], {}], [1, "athlinks-com", 1, "^https://(www\\.)?athlinks\\.com/", 22, [2901], [{ e: 2901 }], [{ v: 2902 }], [{ h: 2901 }], [], {}], [1, "att", 2, "^https://([a-z0-9-]+\\.)*att\\.com/", 22, [2903], [{ exists: ["#gpc-banner-container", "#declineSave"] }], [{ visible: ["#gpc-banner-container", "#declineSave"] }], [{ waitForThenClick: ["#gpc-banner-container", "#declineSave"] }], [{ wait: 1e3 }, { cc: 2904 }], {}], [1, "auctionet", 2, "^https?://(www\\.)?auctionet\\.(com|se)/", 22, [2905], [{ e: 2906 }], [{ v: 2906 }], [{ c: 2907 }, { c: 2908 }], [{ wait: 500 }, { cc: 2909 }], {}], [1, "auto_AU_acetool.com_n0w_+1", 0, "^https?://(www\\.)?acetool\\.com/|^https?://(www\\.)?unclewiener\\.com/", 10, [], [{ e: 2910 }], [{ v: 2910 }], [{ wait: 500 }, { c: 2910 }], [{ timeout: 1e3, check: "none", wv: 2910 }], {}], [1, "auto_AU_aelfie.com_chb", 0, "^https?://(www\\.)?aelfie\\.com/", 10, [], [{ e: 2911 }], [{ v: 2911 }], [{ wait: 500 }, { c: 2911 }], [{ timeout: 1e3, check: "none", wv: 2911 }], {}], [1, "auto_AU_cam.start.canon_3x5", 0, "^https?://(www\\.)?cam\\.start\\.canon/", 10, [], [{ e: 2912 }], [{ v: 2912 }], [{ wait: 500 }, { c: 2912 }], [{ timeout: 1e3, check: "none", wv: 2912 }], {}], [1, "auto_AU_cam.start.canon_z06", 0, "^https?://(www\\.)?cam\\.start\\.canon/", 10, [], [{ e: 2913 }], [{ v: 2913 }], [{ wait: 500 }, { c: 2913 }], [{ timeout: 1e3, check: "none", wv: 2913 }], {}], [1, "auto_AU_capitoltrades.com_ak9", 0, "^https?://(www\\.)?capitoltrades\\.com/", 10, [], [{ e: 2914 }], [{ v: 2914 }], [{ wait: 500 }, { c: 2914 }], [{ timeout: 1e3, check: "none", wv: 2914 }], {}], [1, "auto_AU_community.dyson.com_lek", 0, "^https?://(www\\.)?community\\.dyson\\.com/", 10, [], [{ e: 2915 }], [{ v: 2915 }], [{ wait: 500 }, { c: 2915 }], [{ timeout: 1e3, check: "none", wv: 2915 }], {}], [1, "auto_AU_conference-board.org_dce", 0, "^https?://(www\\.)?conference-board\\.org/", 10, [], [{ e: 2916 }], [{ v: 2916 }], [{ wait: 500 }, { c: 2916 }], [{ timeout: 1e3, check: "none", wv: 2916 }], {}], [1, "auto_AU_cutterbuck.com_ko7", 0, "^https?://(www\\.)?cutterbuck\\.com/", 10, [], [{ e: 2917 }], [{ v: 2917 }], [{ wait: 500 }, { c: 2917 }], [{ timeout: 1e3, check: "none", wv: 2917 }], {}], [1, "auto_AU_deezer.com_tmf", 0, "^https?://(www\\.)?deezer\\.com/", 10, [], [{ e: 2918 }], [{ v: 2918 }], [{ wait: 500 }, { c: 2918 }], [{ timeout: 1e3, check: "none", wv: 2918 }], {}], [1, "auto_AU_docs.portainer.io_hn5", 0, "^https?://(www\\.)?docs\\.portainer\\.io/", 10, [], [{ e: 2919 }], [{ v: 2919 }], [{ wait: 500 }, { c: 2919 }], [{ timeout: 1e3, check: "none", wv: 2919 }], {}], [1, "auto_AU_flinders.edu.au_7en_+1", 0, "^https?://(www\\.)?flinders\\.edu\\.au/|^https?://(www\\.)?students\\.flinders\\.edu\\.au/", 10, [], [{ e: 2920 }], [{ v: 2920 }], [{ c: 2920 }], [], {}], [1, "auto_AU_flysaa.com_qsm", 0, "^https?://(www\\.)?flysaa\\.com/", 10, [2921], [{ e: 2921 }], [{ v: 2922 }], [{ c: 2923 }], [{ timeout: 1e3, check: "none", wv: 2922 }], {}], [1, "auto_AU_gayseniordating.com_bdr", 0, "^https?://(www\\.)?gayseniordating\\.com/", 10, [], [{ e: 2924 }], [{ v: 2924 }], [{ wait: 500 }, { c: 2924 }], [{ timeout: 1e3, check: "none", wv: 2924 }], {}], [1, "auto_AU_goodmoodprints.com_9fy", 1, "^https?://(www\\.)?goodmoodprints\\.com/", 10, [2925], [{ e: 2925 }], [{ v: 2925 }], [{ h: 2925 }], [{ timeout: 1e3, check: "none", wv: 2925 }], {}], [1, "auto_AU_lush.com_bdf", 0, "^https?://(www\\.)?lush\\.com/", 10, [], [{ e: 2926 }], [{ v: 2926 }], [{ wait: 500 }, { c: 2926 }], [{ timeout: 1e3, check: "none", wv: 2926 }], {}], [1, "auto_AU_newyork.doverstreetmarket.com_j5q", 0, "^https?://(www\\.)?newyork\\.doverstreetmarket\\.com/", 10, [], [{ e: 2927 }], [{ v: 2927 }], [{ wait: 500 }, { c: 2927 }], [{ timeout: 1e3, check: "none", wv: 2927 }], {}], [1, "auto_AU_newyork.doverstreetmarket.com_k2q", 0, "^https?://(www\\.)?newyork\\.doverstreetmarket\\.com/", 10, [], [{ e: 2928 }], [{ v: 2928 }], [{ wait: 500 }, { c: 2928 }], [{ timeout: 1e3, check: "none", wv: 2928 }], {}], [1, "auto_AU_niwaki.com_w41", 0, "^https?://(www\\.)?niwaki\\.com/", 10, [], [{ e: 2929 }], [{ v: 2929 }], [{ wait: 500 }, { c: 2929 }], [{ timeout: 1e3, check: "none", wv: 2929 }], {}], [1, "auto_AU_pichunter.com_zhf", 0, "^https?://(www\\.)?pichunter\\.com/", 10, [], [{ e: 2930 }], [{ v: 2930 }], [{ wait: 500 }, { c: 2930 }], [{ timeout: 1e3, check: "none", wv: 2930 }], {}], [1, "auto_AU_pnp.co.za_9lg", 0, "^https?://(www\\.)?pnp\\.co\\.za/", 10, [], [{ e: 2931 }], [{ v: 2931 }], [{ wait: 500 }, { c: 2931 }], [{ timeout: 1e3, check: "none", wv: 2931 }], {}], [1, "auto_AU_publicdomainreview.org_7fg", 0, "^https?://(www\\.)?publicdomainreview\\.org/", 10, [], [{ e: 2932 }], [{ v: 2932 }], [{ wait: 500 }, { c: 2932 }], [{ timeout: 1e3, check: "none", wv: 2932 }], {}], [1, "auto_AU_qldnaturistassoc.org_k53", 0, "^https?://(www\\.)?qldnaturistassoc\\.org/", 10, [], [{ e: 2933 }], [{ v: 2933 }], [{ wait: 500 }, { c: 2933 }], [{ timeout: 1e3, check: "none", wv: 2933 }], {}], [1, "auto_AU_rki.de_tp5", 0, "^https?://(www\\.)?rki\\.de/", 10, [], [{ e: 2934 }], [{ v: 2934 }], [{ wait: 500 }, { c: 2934 }], [{ timeout: 1e3, check: "none", wv: 2934 }], {}], [1, "auto_AU_solesavy.com_8q6", 0, "^https?://(www\\.)?solesavy\\.com/", 10, [], [{ e: 2935 }], [{ v: 2935 }], [{ wait: 500 }, { c: 2935 }], [{ timeout: 1e3, check: "none", wv: 2935 }], {}], [1, "auto_AU_staff.flinders.edu.au_cpu", 0, "^https?://(www\\.)?staff\\.flinders\\.edu\\.au/", 10, [], [{ e: 2920 }], [{ v: 2920 }], [{ wait: 500 }, { c: 2920 }], [{ timeout: 1e3, check: "none", wv: 2920 }], {}], [1, "auto_AU_telekom.hu_4lu", 0, "^https?://(www\\.)?telekom\\.hu/", 10, [], [{ e: 2936 }], [{ v: 2936 }], [{ wait: 500 }, { c: 2936 }], [{ timeout: 1e3, check: "none", wv: 2936 }], {}], [1, "auto_AU_tourismnewbrunswick.ca_bjd", 0, "^https?://(www\\.)?tourismnewbrunswick\\.ca/", 10, [], [{ e: 2937 }], [{ v: 2937 }], [{ wait: 500 }, { c: 2937 }], [{ timeout: 1e3, check: "none", wv: 2937 }], {}], [1, "auto_AU_tripcentral.ca_v7v", 0, "^https?://(www\\.)?tripcentral\\.ca/", 10, [], [{ e: 2938 }], [{ v: 2938 }], [{ wait: 500 }, { c: 2938 }], [{ timeout: 1e3, check: "none", wv: 2938 }], {}], [1, "auto_AU_u-buy.com.au_rg0", 0, "^https?://(www\\.)?u-buy\\.com\\.au/", 10, [], [{ e: 2939 }], [{ v: 2939 }], [{ wait: 500 }, { c: 2939 }], [{ timeout: 1e3, check: "none", wv: 2939 }], {}], [1, "auto_AU_whitepages.co.com_4cs", 0, "^https?://(www\\.)?whitepages\\.co\\.com/", 10, [], [{ e: 2940 }], [{ v: 2940 }], [{ wait: 500 }, { c: 2940 }], [{ timeout: 1e3, check: "none", wv: 2940 }], {}], [1, "auto_CA_407etr.com_wrd", 0, "^https?://(www\\.)?407etr\\.com/", 10, [], [{ e: 2941 }], [{ v: 2941 }], [{ wait: 500 }, { c: 2941 }], [{ timeout: 1e3, check: "none", wv: 2941 }], {}], [1, "auto_CA_arte.tv_7nv", 0, "^https?://(www\\.)?arte\\.tv/", 10, [], [{ e: 2942 }], [{ v: 2942 }], [{ c: 2942 }], [], {}], [1, "auto_CA_babel.hathitrust.org_i6g", 0, "^https?://(www\\.)?babel\\.hathitrust\\.org/", 10, [], [{ e: 2943 }], [{ v: 2943 }], [{ wait: 500 }, { c: 2943 }], [{ timeout: 1e3, check: "none", wv: 2943 }], {}], [1, "auto_CA_belairdirect.com_wic", 0, "^https?://(www\\.)?belairdirect\\.com/", 10, [], [{ e: 2944 }], [{ v: 2944 }], [{ wait: 500 }, { c: 2944 }], [{ timeout: 1e3, check: "none", wv: 2944 }], {}], [1, "auto_CA_bet365.bet.br_kkx", 0, "^https?://(www\\.)?bet365\\.bet\\.br/", 10, [], [{ e: 2945 }], [{ v: 2945 }], [{ wait: 500 }, { c: 2945 }], [{ timeout: 1e3, check: "none", wv: 2945 }], {}], [1, "auto_CA_blackdiamondequipment.com_dwp_+8", 0, "^https?://(www\\.)?blackdiamondequipment\\.com/|^https?://(www\\.)?facetofacegames\\.com/|^https?://(www\\.)?suzyshier\\.com/|^https?://(www\\.)?urban-planet\\.com/|^https?://(www\\.)?eu\\.blackdiamondequipment\\.com/|^https?://(www\\.)?bushwear\\.co\\.uk/|^https?://(www\\.)?directdoors\\.com/|^https?://(www\\.)?keenfootwear\\.com/|^https?://(www\\.)?shop\\.panasonic\\.com/", 10, [], [{ e: 2946 }], [{ v: 2946 }], [{ wait: 500 }, { c: 2946 }], [{ timeout: 1e3, check: "none", wv: 2946 }], {}], [1, "auto_CA_brazzers.com_kik", 0, "^https?://(www\\.)?brazzers\\.com/", 10, [], [{ e: 2947 }], [{ v: 2947 }], [{ wait: 500 }, { c: 2947 }], [{ timeout: 1e3, check: "none", wv: 2947 }], {}], [1, "auto_CA_busbud.com_0sh", 0, "^https?://(www\\.)?busbud\\.com/", 10, [], [{ e: 2948 }], [{ v: 2948 }], [{ wait: 500 }, { c: 2948 }], [{ timeout: 1e3, check: "none", wv: 2948 }], {}], [1, "auto_CA_cegeplimoilou.ca_9tg", 0, "^https?://(www\\.)?cegeplimoilou\\.ca/", 10, [], [{ e: 2949 }], [{ v: 2949 }], [{ wait: 500 }, { c: 2949 }], [{ timeout: 1e3, check: "none", wv: 2949 }], {}], [1, "auto_CA_cfo.coop_edx", 0, "^https?://(www\\.)?cfo\\.coop/", 10, [], [{ e: 2950 }], [{ v: 2950 }], [{ wait: 500 }, { c: 2950 }], [{ timeout: 1e3, check: "none", wv: 2950 }], {}], [1, "auto_CA_chipolo.net_zo9", 0, "^https?://(www\\.)?chipolo\\.net/", 10, [], [{ e: 2951 }], [{ v: 2951 }], [{ wait: 500 }, { c: 2951 }], [{ timeout: 1e3, check: "none", wv: 2951 }], {}], [1, "auto_CA_cmpa-acpm.ca_nja", 0, "^https?://(www\\.)?cmpa-acpm\\.ca/", 10, [], [{ e: 2952 }], [{ v: 2952 }], [{ c: 2952 }], [], {}], [1, "auto_CA_denniskirk.com_ezr", 0, "^https?://(www\\.)?denniskirk\\.com/", 10, [], [{ e: 2953 }], [{ v: 2953 }], [{ wait: 500 }, { c: 2953 }], [{ timeout: 1e3, check: "none", wv: 2953 }], {}], [1, "auto_CA_doyondespres.com_4jr", 0, "^https?://(www\\.)?doyondespres\\.com/", 10, [], [{ e: 2954 }], [{ v: 2954 }], [{ wait: 500 }, { c: 2954 }], [{ timeout: 1e3, check: "none", wv: 2954 }], {}], [1, "auto_CA_f6s.com_yjw", 0, "^https?://(www\\.)?f6s\\.com/", 10, [], [{ e: 2955 }], [{ v: 2955 }], [{ wait: 500 }, { c: 2955 }], [{ timeout: 1e3, check: "none", wv: 2955 }], {}], [1, "auto_CA_fischersports.com_1yr", 0, "^https?://(www\\.)?fischersports\\.com/", 10, [], [{ e: 2956 }], [{ v: 2956 }], [{ wait: 500 }, { c: 2956 }], [{ timeout: 1e3, check: "none", wv: 2956 }], {}], [1, "auto_CA_golfavenue.ca_ero_+1", 0, "^https?://(www\\.)?golfavenue\\.ca/|^https?://(www\\.)?golfbidder\\.co\\.uk/", 10, [], [{ e: 2957 }], [{ v: 2957 }], [{ wait: 500 }, { c: 2957 }], [{ timeout: 1e3, check: "none", wv: 2957 }], {}], [1, "auto_CA_golftown.com_zcn", 0, "^https?://(www\\.)?golftown\\.com/", 10, [], [{ e: 2958 }], [{ v: 2958 }], [{ wait: 500 }, { c: 2958 }], [{ timeout: 1e3, check: "none", wv: 2958 }], {}], [1, "auto_CA_intact.ca_gax", 0, "^https?://(www\\.)?intact\\.ca/", 10, [], [{ e: 2959 }], [{ v: 2959 }], [{ wait: 500 }, { c: 2959 }], [{ timeout: 1e3, check: "none", wv: 2959 }], {}], [1, "auto_CA_lethpolytech.ca_1r1", 0, "^https?://(www\\.)?lethpolytech\\.ca/", 10, [], [{ e: 2960 }], [{ v: 2960 }], [{ wait: 500 }, { c: 2960 }], [{ timeout: 1e3, check: "none", wv: 2960 }], {}], [1, "auto_CA_libgen.help_7s8", 0, "^https?://(www\\.)?libgen\\.help/", 10, [], [{ e: 2961 }], [{ v: 2961 }], [{ wait: 500 }, { c: 2961 }], [{ timeout: 1e3, check: "none", wv: 2961 }], {}], [1, "auto_CA_nationalarchives.gov.uk_vgx", 0, "^https?://(www\\.)?webarchive\\.nationalarchives\\.gov\\.uk/", 10, [], [{ e: 2962 }], [{ v: 2962 }], [{ wait: 500 }, { c: 2962 }], [{ timeout: 1e3, check: "none", wv: 2962 }], {}], [1, "auto_CA_papajohns.ca_n5j", 0, "^https?://(www\\.)?papajohns\\.ca/", 10, [], [{ e: 2963 }], [{ v: 2963 }], [{ wait: 500 }, { c: 2963 }], [{ timeout: 1e3, check: "none", wv: 2963 }], {}], [1, "auto_CA_pcsupport.lenovo.com_dxi", 0, "^https?://(www\\.)?pcsupport\\.lenovo\\.com/", 10, [], [{ e: 2964 }], [{ v: 2964 }], [{ wait: 500 }, { c: 2964 }], [{ timeout: 1e3, check: "none", wv: 2964 }], {}], [1, "auto_CA_queerty.com_fb8_+1", 0, "^https?://(www\\.)?queerty\\.com/|^https?://(www\\.)?pestor\\.nl/", 10, [], [{ e: 2965 }], [{ v: 2965 }], [{ wait: 500 }, { c: 2965 }], [{ timeout: 1e3, check: "none", wv: 2965 }], {}], [1, "auto_CA_remitly.com_mm4", 0, "^https?://(www\\.)?remitly\\.com/", 10, [], [{ e: 2966 }], [{ v: 2966 }], [{ wait: 500 }, { c: 2966 }], [{ timeout: 1e3, check: "none", wv: 2966 }], {}], [1, "auto_CA_reviewed.com_w2k", 0, "^https?://(www\\.)?reviewed\\.com/", 10, [], [{ e: 2967 }], [{ v: 2967 }], [{ wait: 500 }, { c: 2967 }], [{ timeout: 1e3, check: "none", wv: 2967 }], {}], [1, "auto_CA_sheetmusicplus.com_b7d", 0, "^https?://(www\\.)?sheetmusicplus\\.com/", 10, [], [{ e: 2968 }], [{ v: 2968 }], [{ wait: 500 }, { c: 2968 }], [{ timeout: 1e3, check: "none", wv: 2968 }], {}], [1, "auto_CA_skipthedishes.com_1sg", 0, "^https?://(www\\.)?skipthedishes\\.com/", 10, [], [{ e: 2969 }], [{ v: 2969 }], [{ c: 2969 }], [], {}], [1, "auto_CA_streamingthe.net_v8e", 0, "^https?://(www\\.)?streamingthe\\.net/", 10, [], [{ e: 2970 }], [{ v: 2970 }], [{ wait: 500 }, { c: 2970 }], [{ timeout: 1e3, check: "none", wv: 2970 }], {}], [1, "auto_CA_ticketsource.com_kiv", 0, "^https?://(www\\.)?ticketsource\\.com/", 10, [], [{ e: 2971 }], [{ v: 2971 }], [{ wait: 500 }, { c: 2971 }], [{ timeout: 1e3, check: "none", wv: 2971 }], {}], [1, "auto_CA_ubereats.com_4nx", 0, "^https?://(www\\.)?ubereats\\.com/", 10, [], [{ e: 2972 }], [{ v: 2972 }], [{ wait: 500 }, { c: 2972 }], [{ timeout: 1e3, check: "none", wv: 2972 }], {}], [1, "auto_CA_wealthsimple.com_kgz", 0, "^https?://(www\\.)?wealthsimple\\.com/", 10, [], [{ e: 2973 }], [{ v: 2973 }], [{ wait: 500 }, { c: 2973 }], [{ timeout: 1e3, check: "none", wv: 2973 }], {}], [1, "auto_CA_xplore.ca_wwx", 0, "^https?://(www\\.)?xplore\\.ca/", 10, [], [{ e: 2974 }], [{ v: 2974 }], [{ wait: 500 }, { c: 2974 }], [{ timeout: 1e3, check: "none", wv: 2974 }], {}], [1, "auto_CH_3djake.ch_6k2_+3", 0, "^https?://(www\\.)?3djake\\.ch/|^https?://(www\\.)?ecco-verde\\.ch/|^https?://(www\\.)?vitalabo\\.ch/|^https?://(www\\.)?3djake\\.de/", 10, [], [{ e: 2975 }], [{ v: 2975 }], [{ wait: 500 }, { c: 2975 }], [{ timeout: 1e3, check: "none", wv: 2975 }], {}], [1, "auto_CH_alpenvereinaktiv.com_ms8_+1", 0, "^https?://(www\\.)?alpenvereinaktiv\\.com/|^https?://(www\\.)?maps\\.viamala\\.ch/", 10, [], [{ e: 2976 }], [{ v: 2976 }], [{ wait: 500 }, { c: 2976 }], [{ timeout: 1e3, check: "none", wv: 2976 }], {}], [1, "auto_CH_android.bestsecret.com_m12_+2", 0, "^https?://(www\\.)?android\\.bestsecret\\.com/|^https?://(www\\.)?bestsecret\\.com/|^https?://(www\\.)?orders\\.bestsecret\\.com/", 10, [], [{ e: 2977 }], [{ v: 2977 }], [{ wait: 500 }, { c: 2977 }], [{ timeout: 1e3, check: "none", wv: 2977 }], {}], [1, "auto_CH_aoc.com_dib", 0, "^https?://(www\\.)?aoc\\.com/", 10, [], [{ e: 2978 }], [{ v: 2978 }], [{ wait: 500 }, { c: 2978 }], [{ timeout: 1e3, check: "none", wv: 2978 }], {}], [1, "auto_CH_arte.tv_ln7", 0, "^https?://(www\\.)?arte\\.tv/", 10, [], [{ e: 2979 }], [{ v: 2979 }], [{ wait: 500 }, { c: 2979 }], [{ timeout: 1e3, check: "none", wv: 2979 }], {}], [1, "auto_CH_arttv.ch_2lm", 0, "^https?://(www\\.)?arttv\\.ch/", 10, [], [{ e: 2980 }], [{ v: 2980 }], [{ wait: 500 }, { c: 2980 }], [{ timeout: 1e3, check: "none", wv: 2980 }], {}], [1, "auto_CH_ascona.il-centro.ch_59k", 0, "^https?://(www\\.)?ascona\\.il-centro\\.ch/", 10, [], [{ e: 2981 }], [{ v: 2981 }], [{ wait: 500 }, { c: 2981 }], [{ timeout: 1e3, check: "none", wv: 2981 }], {}], [1, "auto_CH_astag.ch_1if", 0, "^https?://(www\\.)?astag\\.ch/", 10, [], [{ e: 2982 }], [{ v: 2982 }], [{ wait: 500 }, { c: 2982 }], [{ timeout: 1e3, check: "none", wv: 2982 }], {}], [1, "auto_CH_auto.swiss_z35", 0, "^https?://(www\\.)?auto\\.swiss/", 10, [], [{ e: 2983 }], [{ v: 2983 }], [{ wait: 500 }, { c: 2983 }], [{ timeout: 1e3, check: "none", wv: 2983 }], {}], [1, "auto_CH_bandlab.com_jul", 0, "^https?://(www\\.)?bandlab\\.com/", 10, [], [{ e: 2984 }], [{ v: 2984 }], [{ wait: 500 }, { c: 2984 }], [{ timeout: 1e3, check: "none", wv: 2984 }], {}], [1, "auto_CH_bitbox.swiss_w0n", 0, "^https?://(www\\.)?bitbox\\.swiss/", 10, [], [{ e: 2985 }], [{ v: 2985 }], [{ wait: 500 }, { c: 2985 }], [{ timeout: 1e3, check: "none", wv: 2985 }], {}], [1, "auto_CH_blogdumoderateur.com_8p6", 0, "^https?://(www\\.)?blogdumoderateur\\.com/", 10, [], [{ e: 2986 }], [{ v: 2986 }], [{ wait: 500 }, { c: 2986 }], [{ timeout: 1e3, check: "none", wv: 2986 }], {}], [1, "auto_CH_blutspendezurich.ch_s0f", 0, "^https?://(www\\.)?blutspendezurich\\.ch/", 10, [], [{ e: 2987 }], [{ v: 2987 }], [{ wait: 500 }, { c: 2987 }], [{ timeout: 1e3, check: "none", wv: 2987 }], {}], [1, "auto_CH_cardmarket.com_15t", 0, "^https?://(www\\.)?cardmarket\\.com/", 10, [], [{ e: 2988 }], [{ v: 2988 }], [{ wait: 500 }, { c: 2988 }], [{ timeout: 1e3, check: "none", wv: 2988 }], {}], [1, "auto_CH_ch.rotho.com_h99_+2", 0, "^https?://(www\\.)?ch\\.rotho\\.com/|^https?://(www\\.)?reclam\\.de/|^https?://(www\\.)?dacianer\\.de/", 10, [], [{ e: 2989 }], [{ v: 2989 }], [{ wait: 500 }, { c: 2989 }], [{ timeout: 1e3, check: "none", wv: 2989 }], {}], [1, "auto_CH_chrono24.ch_dve_+1", 0, "^https?://(www\\.)?chrono24\\.ch/|^https?://(www\\.)?chrono24\\.de/", 10, [], [{ e: 2990 }], [{ v: 2990 }], [{ wait: 500 }, { c: 2990 }], [{ timeout: 1e3, check: "none", wv: 2990 }], {}], [1, "auto_CH_chrono24.com_lj5_+1", 0, "^https?://(www\\.)?chrono24\\.com/|^https?://(www\\.)?chrono24\\.co\\.uk/", 10, [], [{ e: 2990 }], [{ v: 2990 }], [{ wait: 500 }, { c: 2990 }], [{ timeout: 1e3, check: "none", wv: 2990 }], {}], [1, "auto_CH_ersatzteil-shop24.de_mh5_+1", 0, "^https?://(www\\.)?ersatzteil-shop24\\.de/|^https?://(www\\.)?der-rasenmaeher\\.de/", 10, [], [{ e: 2991 }], [{ v: 2991 }], [{ wait: 500 }, { c: 2991 }], [{ timeout: 1e3, check: "none", wv: 2991 }], {}], [1, "auto_CH_europarl.europa.eu_y2d", 0, "^https?://(www\\.)?europarl\\.europa\\.eu/", 10, [], [{ e: 2992 }], [{ v: 2992 }], [{ wait: 500 }, { c: 2992 }], [{ timeout: 1e3, check: "none", wv: 2992 }], {}], [1, "auto_CH_feldschloesschen.ch_4mc", 0, "^https?://(www\\.)?feldschloesschen\\.ch/", 10, [], [{ e: 2993 }], [{ v: 2993 }], [{ wait: 500 }, { c: 2993 }], [{ timeout: 1e3, check: "none", wv: 2993 }], {}], [1, "auto_CH_fhgr.ch_9nr", 0, "^https?://(www\\.)?fhgr\\.ch/", 10, [], [{ e: 2994 }], [{ v: 2994 }], [{ wait: 500 }, { c: 2994 }], [{ timeout: 1e3, check: "none", wv: 2994 }], {}], [1, "auto_CH_fondationbeyeler.ch_o13", 0, "^https?://(www\\.)?fondationbeyeler\\.ch/", 10, [], [{ e: 2989 }], [{ v: 2989 }], [{ wait: 500 }, { c: 2989 }], [{ timeout: 1e3, check: "none", wv: 2989 }], {}], [1, "auto_CH_fontis-shop.ch_ksd", 0, "^https?://(www\\.)?fontis-shop\\.ch/", 10, [], [{ e: 2995 }], [{ v: 2995 }], [{ wait: 500 }, { c: 2995 }], [{ timeout: 1e3, check: "none", wv: 2995 }], {}], [1, "auto_CH_frankenspalter.ch_jo0", 0, "^https?://(www\\.)?frankenspalter\\.ch/", 10, [], [{ e: 2996 }], [{ v: 2996 }], [{ wait: 500 }, { c: 2996 }], [{ timeout: 1e3, check: "none", wv: 2996 }], {}], [1, "auto_CH_frankenspalter.ch_l2w", 0, "^https?://(www\\.)?frankenspalter\\.ch/", 10, [], [{ e: 2997 }], [{ v: 2997 }], [{ wait: 500 }, { c: 2997 }], [{ timeout: 1e3, check: "none", wv: 2997 }], {}], [1, "auto_CH_fruugoschweiz.com_6k8", 0, "^https?://(www\\.)?fruugoschweiz\\.com/", 10, [], [{ e: 2998 }], [{ v: 2998 }], [{ wait: 500 }, { c: 2998 }], [{ timeout: 1e3, check: "none", wv: 2998 }], {}], [1, "auto_CH_gabrielweinberg.com_56i", 0, "^https?://(www\\.)?gabrielweinberg\\.com/", 10, [], [{ e: 2999 }], [{ v: 2999 }], [{ wait: 500 }, { c: 2999 }], [{ timeout: 1e3, check: "none", wv: 2999 }], {}], [1, "auto_CH_galaxus.de_sqv_+1", 0, "^https?://(www\\.)?galaxus\\.de/|^https?://(www\\.)?galaxus\\.fr/", 10, [], [{ e: 3e3 }], [{ v: 3e3 }], [{ wait: 500 }, { c: 3e3 }], [{ timeout: 1e3, check: "none", wv: 3e3 }], {}], [1, "auto_CH_gstaadmenuhinfestival.ch_vp3", 0, "^https?://(www\\.)?gstaadmenuhinfestival\\.ch/", 10, [], [{ e: 3001 }], [{ v: 3001 }], [{ wait: 500 }, { c: 3001 }], [{ timeout: 1e3, check: "none", wv: 3001 }], {}], [1, "auto_CH_haushaltstipps.com_9kx_+1", 0, "^https?://(www\\.)?haushaltstipps\\.com/|^https?://(www\\.)?alltours\\.de/", 10, [], [{ e: 2989 }], [{ v: 2989 }], [{ wait: 500 }, { c: 2989 }], [{ timeout: 1e3, check: "none", wv: 2989 }], {}], [1, "auto_CH_independentxpress.de_0qj", 0, "^https?://(www\\.)?independentxpress\\.de/", 10, [], [{ e: 3002 }], [{ v: 3002 }], [{ wait: 500 }, { c: 3002 }], [{ timeout: 1e3, check: "none", wv: 3002 }], {}], [1, "auto_CH_instant-gaming.com_8j3", 0, "^https?://(www\\.)?instant-gaming\\.com/", 10, [], [{ e: 3003 }], [{ v: 3003 }], [{ wait: 500 }, { c: 3003 }], [{ timeout: 1e3, check: "none", wv: 3003 }], {}], [1, "auto_CH_kinopoisk.ru_w3g_+14", 0, "^https?://(www\\.)?kinopoisk\\.ru/|^https?://(www\\.)?market\\.yandex\\.ru/|^https?://(www\\.)?translate\\.yandex\\.com/|^https?://(www\\.)?yandex\\.com\\.tr/|^https?://(www\\.)?ya\\.ru/|^https?://(www\\.)?yandex\\.com/|^https?://(www\\.)?translate\\.yandex\\.ru/|^https?://(www\\.)?yandex\\.by/|^https?://(www\\.)?local\\.yandex\\.com/|^https?://(www\\.)?sso\\.passport\\.yandex\\.ru/|^https?://(www\\.)?360\\.yandex\\.ru/|^https?://(www\\.)?360\\.yandex\\.com/|^https?://(www\\.)?music\\.yandex\\.ru/|^https?://(www\\.)?hd\\.kinopoisk\\.ru/|^https?://(www\\.)?wap\\.yandex\\.com/", 10, [], [{ e: 3004 }], [{ v: 3004 }], [{ wait: 500 }, { c: 3004 }], [{ timeout: 1e3, check: "none", wv: 3004 }], {}], [1, "auto_CH_kobo.com_p4l", 0, "^https?://(www\\.)?kobo\\.com/", 10, [], [{ e: 3005 }], [{ v: 3005 }], [{ wait: 500 }, { c: 3005 }], [{ timeout: 1e3, check: "none", wv: 3005 }], {}], [1, "auto_CH_labanquepostale.fr_o6f_+1", 0, "^https?://(www\\.)?labanquepostale\\.fr/|^https?://(www\\.)?labanquepostale\\.com/", 10, [], [{ e: 3006 }], [{ v: 3006 }], [{ wait: 500 }, { c: 3006 }], [{ timeout: 1e3, check: "none", wv: 3006 }], {}], [1, "auto_CH_laposte.fr_ctu_+2", 0, "^https?://(www\\.)?laposte\\.fr/|^https?://(www\\.)?localiser\\.laposte\\.fr/|^https?://(www\\.)?aide\\.laposte\\.fr/", 10, [], [{ e: 3007 }], [{ v: 3007 }], [{ wait: 500 }, { c: 3007 }], [{ timeout: 1e3, check: "none", wv: 3007 }], {}], [1, "auto_CH_larian.com_kp5", 0, "^https?://(www\\.)?larian\\.com/", 10, [], [{ e: 3008 }], [{ v: 3008 }], [{ wait: 500 }, { c: 3008 }], [{ timeout: 1e3, check: "none", wv: 3008 }], {}], [1, "auto_CH_louis-moto.ch_5yk", 0, "^https?://(www\\.)?louis-moto\\.ch/", 10, [], [{ e: 3009 }], [{ v: 3009 }], [{ wait: 500 }, { c: 3009 }], [{ timeout: 1e3, check: "none", wv: 3009 }], {}], [1, "auto_CH_manufactum.ch_ag0_+1", 0, "^https?://(www\\.)?manufactum\\.ch/|^https?://(www\\.)?manufactum\\.de/", 10, [], [{ e: 3010 }], [{ v: 3010 }], [{ wait: 500 }, { c: 3010 }], [{ timeout: 1e3, check: "none", wv: 3010 }], {}], [1, "auto_CH_maps.engadin.ch_m40_+3", 0, "^https?://(www\\.)?maps\\.engadin\\.ch/|^https?://(www\\.)?outdoor\\.glarnerland\\.ch/|^https?://(www\\.)?altenberg\\.de/|^https?://(www\\.)?tourenplaner-rheinland-pfalz\\.de/", 10, [], [{ e: 2976 }], [{ v: 2976 }], [{ wait: 500 }, { c: 2976 }], [{ timeout: 1e3, check: "none", wv: 2976 }], {}], [1, "auto_CH_mein-kraeuterkeller.de_zjh", 0, "^https?://(www\\.)?mein-kraeuterkeller\\.de/", 10, [], [{ e: 3011 }], [{ v: 3011 }], [{ wait: 500 }, { c: 3011 }], [{ timeout: 1e3, check: "none", wv: 3011 }], {}], [1, "auto_CH_meintiptopf.ch_84l", 0, "^https?://(www\\.)?meintiptopf\\.ch/", 10, [], [{ e: 3012 }], [{ v: 3012 }], [{ wait: 500 }, { c: 3012 }], [{ timeout: 1e3, check: "none", wv: 3012 }], {}], [1, "auto_CH_mio.se_hd0", 0, "^https?://(www\\.)?mio\\.se/", 10, [], [{ e: 3013 }], [{ v: 3013 }], [{ wait: 500 }, { c: 3013 }], [{ timeout: 1e3, check: "none", wv: 3013 }], {}], [1, "auto_CH_mrporter.com_0nl_+1", 0, "^https?://(www\\.)?mrporter\\.com/|^https?://(www\\.)?net-a-porter\\.com/", 10, [], [{ e: 3014 }], [{ v: 3014 }], [{ wait: 500 }, { c: 3014 }], [{ timeout: 1e3, check: "none", wv: 3014 }], {}], [1, "auto_CH_platform.openai.com_g5y", 0, "^https?://(www\\.)?platform\\.openai\\.com/", 10, [], [{ e: 3015 }], [{ v: 3015 }], [{ wait: 500 }, { c: 3015 }], [{ timeout: 1e3, check: "none", wv: 3015 }], {}], [1, "auto_CH_pmphotomedia.ch_ume", 0, "^https?://(www\\.)?pmphotomedia\\.ch/", 10, [], [{ e: 3016 }], [{ v: 3016 }], [{ wait: 500 }, { c: 3016 }], [{ timeout: 1e3, check: "none", wv: 3016 }], {}], [1, "auto_CH_radio1.ch_1v2", 0, "^https?://(www\\.)?radio1\\.ch/", 10, [], [{ e: 3017 }], [{ v: 3017 }], [{ wait: 500 }, { c: 3017 }], [{ timeout: 1e3, check: "none", wv: 3017 }], {}], [1, "auto_CH_renens.ch_2uc", 0, "^https?://(www\\.)?renens\\.ch/", 10, [], [{ e: 3018 }], [{ v: 3018 }], [{ wait: 500 }, { c: 3018 }], [{ timeout: 1e3, check: "none", wv: 3018 }], {}], [1, "auto_CH_sac-uto.ch_3gu", 0, "^https?://(www\\.)?sac-uto\\.ch/", 10, [], [{ e: 3019 }], [{ v: 3019 }], [{ wait: 500 }, { c: 3019 }], [{ timeout: 1e3, check: "none", wv: 3019 }], {}], [1, "auto_CH_seetickets.com_4b2", 0, "^https?://(www\\.)?seetickets\\.com/", 10, [], [{ e: 3020 }], [{ v: 3020 }], [{ wait: 500 }, { c: 3020 }], [{ timeout: 1e3, check: "none", wv: 3020 }], {}], [1, "auto_CH_sparkasse.de_lwr", 0, "^https?://(www\\.)?sparkasse\\.de/", 10, [], [{ e: 3021 }], [{ v: 3021 }], [{ wait: 500 }, { c: 3021 }], [{ timeout: 1e3, check: "none", wv: 3021 }], {}], [1, "auto_CH_sva-bl.ch_h2k_+1", 0, "^https?://(www\\.)?sva-bl\\.ch/|^https?://(www\\.)?uni-bremen\\.de/", 10, [], [{ e: 3022 }], [{ v: 3022 }], [{ wait: 500 }, { c: 3022 }], [{ timeout: 1e3, check: "none", wv: 3022 }], {}], [1, "auto_CH_transn.ch_ygb", 0, "^https?://(www\\.)?transn\\.ch/", 10, [], [{ e: 3023 }], [{ v: 3023 }], [{ wait: 500 }, { c: 3023 }], [{ timeout: 1e3, check: "none", wv: 3023 }], {}], [1, "auto_CH_ubs-kidscup.ch_5oc", 0, "^https?://(www\\.)?ubs-kidscup\\.ch/", 10, [], [{ e: 3024 }], [{ v: 3024 }], [{ wait: 500 }, { c: 3024 }], [{ timeout: 1e3, check: "none", wv: 3024 }], {}], [1, "auto_CH_velofactory.ch_tpq", 0, "^https?://(www\\.)?velofactory\\.ch/", 10, [], [{ e: 3025 }], [{ v: 3025 }], [{ wait: 500 }, { c: 3025 }], [{ timeout: 1e3, check: "none", wv: 3025 }], {}], [1, "auto_DE_116117-termine.de_nwg", 0, "^https?://(www\\.)?116117-termine\\.de/", 10, [], [{ e: 3026 }], [{ v: 3026 }], [{ wait: 500 }, { c: 3026 }], [{ timeout: 1e3, check: "none", wv: 3026 }], {}], [1, "auto_DE_6relax.de_na9", 0, "^https?://(www\\.)?6relax\\.de/", 10, [], [{ e: 3027 }], [{ v: 3027 }], [{ wait: 500 }, { c: 3027 }], [{ timeout: 1e3, check: "none", wv: 3027 }], {}], [1, "auto_DE_accio.com_kdu", 0, "^https?://(www\\.)?accio\\.com/", 10, [], [{ e: 3028 }], [{ v: 3028 }], [{ wait: 500 }, { c: 3028 }], [{ timeout: 1e3, check: "none", wv: 3028 }], {}], [1, "auto_DE_aerztekammer-bw.de_rse", 0, "^https?://(www\\.)?aerztekammer-bw\\.de/", 10, [], [{ e: 3029 }], [{ v: 3029 }], [{ wait: 500 }, { c: 3029 }], [{ timeout: 1e3, check: "none", wv: 3029 }], {}], [1, "auto_DE_afd.de_tad", 0, "^https?://(www\\.)?afd\\.de/", 10, [], [{ e: 3030 }], [{ v: 3030 }], [{ wait: 500 }, { c: 3030 }], [{ timeout: 1e3, check: "none", wv: 3030 }], {}], [1, "auto_DE_aknw.de_wcz_+1", 0, "^https?://(www\\.)?aknw\\.de/|^https?://(www\\.)?regioentsorgung\\.de/", 10, [], [{ e: 3031 }], [{ v: 3031 }], [{ wait: 500 }, { c: 3031 }], [{ timeout: 1e3, check: "none", wv: 3031 }], {}], [1, "auto_DE_all-inkl.com_toh", 0, "^https?://(www\\.)?all-inkl\\.com/", 10, [], [{ e: 3032 }], [{ v: 3032 }], [{ wait: 500 }, { c: 3032 }], [{ timeout: 1e3, check: "none", wv: 3032 }], {}], [1, "auto_DE_almenrausch.at_c6z", 0, "^https?://(www\\.)?almenrausch\\.at/", 10, [], [{ e: 3033 }], [{ v: 3033 }], [{ wait: 500 }, { c: 3033 }], [{ timeout: 1e3, check: "none", wv: 3033 }], {}], [1, "auto_DE_alza.de_iq3", 0, "^https?://(www\\.)?alza\\.de/", 10, [], [{ e: 3034 }], [{ v: 3034 }], [{ wait: 500 }, { c: 3034 }], [{ timeout: 1e3, check: "none", wv: 3034 }], {}], [1, "auto_DE_ancestry.com_k1k", 0, "^https?://(www\\.)?ancestry\\.com/", 10, [], [{ e: 3035 }], [{ v: 3035 }], [{ wait: 500 }, { c: 3035 }], [{ timeout: 1e3, check: "none", wv: 3035 }], {}], [1, "auto_DE_apozilla.de_d1h", 0, "^https?://(www\\.)?apozilla\\.de/", 10, [], [{ e: 3036 }], [{ v: 3036 }], [{ wait: 500 }, { c: 3036 }], [{ timeout: 1e3, check: "none", wv: 3036 }], {}], [1, "auto_DE_ardplus.de_ue3", 0, "^https?://(www\\.)?ardplus\\.de/", 10, [], [{ e: 3037 }], [{ v: 3037 }], [{ wait: 500 }, { c: 3037 }], [{ timeout: 1e3, check: "none", wv: 3037 }], {}], [1, "auto_DE_axa.de_vyk", 0, "^https?://(www\\.)?axa\\.de/", 10, [], [{ e: 3038 }], [{ v: 3038 }], [{ wait: 500 }, { c: 3038 }], [{ timeout: 1e3, check: "none", wv: 3038 }], {}], [1, "auto_DE_backmarket.de_dzf", 0, "^https?://(www\\.)?backmarket\\.de/", 10, [], [{ e: 3039 }], [{ v: 3039 }], [{ wait: 500 }, { c: 3039 }], [{ timeout: 1e3, check: "none", wv: 3039 }], {}], [1, "auto_DE_bbbank.de_dcf_+15", 0, "^https?://(www\\.)?bbbank\\.de/|^https?://(www\\.)?diebank\\.de/|^https?://(www\\.)?genobroker\\.de/|^https?://(www\\.)?ligabank\\.de/|^https?://(www\\.)?pax-bkc\\.de/|^https?://(www\\.)?psd-berlin-brandenburg\\.de/|^https?://(www\\.)?psd-nuernberg\\.de/|^https?://(www\\.)?sparda-bank-hamburg\\.de/|^https?://(www\\.)?sparda-h\\.de/|^https?://(www\\.)?sparda-n\\.de/|^https?://(www\\.)?v-mn\\.de/|^https?://(www\\.)?vr-bayernmitte\\.de/|^https?://(www\\.)?vrbank-brs\\.de/|^https?://(www\\.)?vrbank-eg\\.de/|^https?://(www\\.)?vrbank-lb\\.de/|^https?://(www\\.)?wvb\\.de/", 10, [], [{ e: 3040 }], [{ v: 3040 }], [{ wait: 500 }, { c: 3040 }], [{ timeout: 1e3, check: "none", wv: 3040 }], {}], [1, "auto_DE_biunsinnorden.de_kwa_+1", 0, "^https?://(www\\.)?biunsinnorden\\.de/|^https?://(www\\.)?livegigs\\.de/", 10, [], [{ e: 3041 }], [{ v: 3041 }], [{ wait: 500 }, { c: 3041 }], [{ timeout: 1e3, check: "none", wv: 3041 }], {}], [1, "auto_DE_bmz.de_nss", 0, "^https?://(www\\.)?bmz\\.de/", 10, [], [{ e: 3042 }], [{ v: 3042 }], [{ wait: 500 }, { c: 3042 }], [{ timeout: 1e3, check: "none", wv: 3042 }], {}], [1, "auto_DE_buerklin.com_bya", 0, "^https?://(www\\.)?buerklin\\.com/", 10, [], [{ e: 2989 }], [{ v: 2989 }], [{ wait: 500 }, { c: 2989 }], [{ timeout: 1e3, check: "none", wv: 2989 }], {}], [1, "auto_DE_bundeswehrkarriere.de_g9g", 0, "^https?://(www\\.)?bundeswehrkarriere\\.de/", 10, [], [{ e: 3043 }], [{ v: 3043 }], [{ wait: 500 }, { c: 3043 }], [{ timeout: 1e3, check: "none", wv: 3043 }], {}], [1, "auto_DE_byak.de_dcj", 0, "^https?://(www\\.)?byak\\.de/", 10, [], [{ e: 3044 }], [{ v: 3044 }], [{ wait: 500 }, { c: 3044 }], [{ timeout: 1e3, check: "none", wv: 3044 }], {}], [1, "auto_DE_byte.fm_83l", 0, "^https?://(www\\.)?byte\\.fm/", 10, [], [{ e: 3045 }], [{ v: 3045 }], [{ wait: 500 }, { c: 3045 }], [{ timeout: 1e3, check: "none", wv: 3045 }], {}], [1, "auto_DE_camping-outdoorshop.de_oo4_+1", 0, "^https?://(www\\.)?camping-outdoorshop\\.de/|^https?://(www\\.)?elektro-wandelt\\.de/", 10, [], [{ e: 3033 }], [{ v: 3033 }], [{ wait: 500 }, { c: 3033 }], [{ timeout: 1e3, check: "none", wv: 3033 }], {}], [1, "auto_DE_club.auto-doc.at_6xj", 0, "^https?://(www\\.)?club\\.auto-doc\\.at/", 10, [], [{ e: 3046 }], [{ v: 3046 }], [{ wait: 500 }, { c: 3046 }], [{ timeout: 1e3, check: "none", wv: 3046 }], {}], [1, "auto_DE_daad.de_w45", 0, "^https?://(www\\.)?daad\\.de/", 10, [], [{ e: 3047 }], [{ v: 3047 }], [{ wait: 500 }, { c: 3047 }], [{ timeout: 1e3, check: "none", wv: 3047 }], {}], [1, "auto_DE_das-ist-drin.de_e12", 0, "^https?://(www\\.)?das-ist-drin\\.de/", 10, [], [{ e: 3048 }], [{ v: 3048 }], [{ wait: 500 }, { c: 3048 }], [{ timeout: 1e3, check: "none", wv: 3048 }], {}], [1, "auto_DE_de.accio.com_97o", 0, "^https?://(www\\.)?de\\.accio\\.com/", 10, [], [{ e: 3028 }], [{ v: 3028 }], [{ wait: 500 }, { c: 3028 }], [{ timeout: 1e3, check: "none", wv: 3028 }], {}], [1, "auto_DE_de.artprice.com_kfk", 0, "^https?://(www\\.)?de\\.artprice\\.com/", 10, [], [{ e: 3049 }], [{ v: 3049 }], [{ wait: 500 }, { c: 3049 }], [{ timeout: 1e3, check: "none", wv: 3049 }], {}], [1, "auto_DE_de.nothing.tech_0fr_+1", 0, "^https?://(www\\.)?de\\.nothing\\.tech/|^https?://(www\\.)?nothing\\.tech/", 10, [], [{ e: 3050 }], [{ v: 3050 }], [{ wait: 500 }, { c: 3050 }], [{ timeout: 1e3, check: "none", wv: 3050 }], {}], [1, "auto_DE_dekra.de_qrb", 0, "^https?://(www\\.)?dekra\\.de/", 10, [], [{ e: 3051 }], [{ v: 3051 }], [{ wait: 500 }, { c: 3051 }], [{ timeout: 1e3, check: "none", wv: 3051 }], {}], [1, "auto_DE_edelstahl-tuerklingel.de_375", 0, "^https?://(www\\.)?edelstahl-tuerklingel\\.de/", 10, [], [{ e: 3052 }], [{ v: 3052 }], [{ wait: 500 }, { c: 3052 }], [{ timeout: 1e3, check: "none", wv: 3052 }], {}], [1, "auto_DE_eezy.nrw_9aj", 0, "^https?://(www\\.)?eezy\\.nrw/", 10, [], [{ e: 3053 }], [{ v: 3053 }], [{ wait: 500 }, { c: 3053 }], [{ timeout: 1e3, check: "none", wv: 3053 }], {}], [1, "auto_DE_ernstings-family.de_xqf", 0, "^https?://(www\\.)?ernstings-family\\.de/", 10, [], [{ e: 3054 }], [{ v: 3054 }], [{ wait: 500 }, { c: 3054 }], [{ timeout: 1e3, check: "none", wv: 3054 }], {}], [1, "auto_DE_fcbinside.de_0d6", 0, "^https?://(www\\.)?fcbinside\\.de/", 10, [], [{ e: 3055 }], [{ v: 3055 }], [{ wait: 500 }, { c: 3055 }], [{ timeout: 1e3, check: "none", wv: 3055 }], {}], [1, "auto_DE_feierabend.de_kr4", 0, "^https?://(www\\.)?feierabend\\.de/", 10, [], [{ e: 3056 }], [{ v: 3056 }], [{ wait: 500 }, { c: 3056 }], [{ timeout: 1e3, check: "none", wv: 3056 }], {}], [1, "auto_DE_feser-graf.de_qz8", 0, "^https?://(www\\.)?feser-graf\\.de/", 10, [], [{ e: 3057 }], [{ v: 3057 }], [{ wait: 500 }, { c: 3057 }], [{ timeout: 1e3, check: "none", wv: 3057 }], {}], [1, "auto_DE_finanzpartner.de_13s", 0, "^https?://(www\\.)?finanzpartner\\.de/", 10, [], [{ e: 3058 }], [{ v: 3058 }], [{ wait: 500 }, { c: 3058 }], [{ timeout: 1e3, check: "none", wv: 3058 }], {}], [1, "auto_DE_gasometer.de_0xw", 0, "^https?://(www\\.)?gasometer\\.de/", 10, [], [{ e: 3059 }], [{ v: 3059 }], [{ wait: 500 }, { c: 3059 }], [{ timeout: 1e3, check: "none", wv: 3059 }], {}], [1, "auto_DE_hermoney.de_jsi", 0, "^https?://(www\\.)?hermoney\\.de/", 10, [], [{ e: 3060 }], [{ v: 3060 }], [{ wait: 500 }, { c: 3060 }], [{ timeout: 1e3, check: "none", wv: 3060 }], {}], [1, "auto_DE_hilfe.kleinanzeigen.de_44a_+1", 0, "^https?://(www\\.)?hilfe\\.kleinanzeigen\\.de/|^https?://(www\\.)?themen\\.kleinanzeigen\\.de/", 10, [], [{ e: 3061 }], [{ v: 3061 }], [{ wait: 500 }, { c: 3061 }], [{ timeout: 1e3, check: "none", wv: 3061 }], {}], [1, "auto_DE_howik.com_99g", 0, "^https?://(www\\.)?howik\\.com/", 10, [], [{ e: 3062 }], [{ v: 3062 }], [{ wait: 500 }, { c: 3062 }], [{ timeout: 1e3, check: "none", wv: 3062 }], {}], [1, "auto_DE_hu-berlin.de_sk6", 0, "^https?://(www\\.)?hu-berlin\\.de/", 10, [], [{ e: 3063 }], [{ v: 3063 }], [{ wait: 500 }, { c: 3063 }], [{ timeout: 1e3, check: "none", wv: 3063 }], {}], [1, "auto_DE_imd-berlin.de_6m1", 0, "^https?://(www\\.)?imd-berlin\\.de/", 10, [], [{ e: 3064 }], [{ v: 3064 }], [{ wait: 500 }, { c: 3064 }], [{ timeout: 1e3, check: "none", wv: 3064 }], {}], [1, "auto_DE_immobilien.sparkasse.de_zj7", 0, "^https?://(www\\.)?immobilien\\.sparkasse\\.de/", 10, [3065], [{ e: 3066 }], [{ v: 3066 }], [{ c: 3067 }], [{ timeout: 1e3, check: "none", wv: 3066 }], {}], [1, "auto_DE_impfen-info.de_am5_+1", 0, "^https?://(www\\.)?impfen-info\\.de/|^https?://(www\\.)?infektionsschutz\\.de/", 10, [], [{ e: 3068 }], [{ v: 3068 }], [{ wait: 500 }, { c: 3068 }], [{ timeout: 1e3, check: "none", wv: 3068 }], {}], [1, "auto_DE_jobvector.de_641", 0, "^https?://(www\\.)?jobvector\\.de/", 10, [], [{ e: 3069 }], [{ v: 3069 }], [{ wait: 500 }, { c: 3069 }], [{ timeout: 1e3, check: "none", wv: 3069 }], {}], [1, "auto_DE_kinsta.com_hc5", 0, "^https?://(www\\.)?kinsta\\.com/", 10, [], [{ e: 3070 }], [{ v: 3070 }], [{ wait: 500 }, { c: 3070 }], [{ timeout: 1e3, check: "none", wv: 3070 }], {}], [1, "auto_DE_kleineskraftwerk.de_fx7", 0, "^https?://(www\\.)?kleineskraftwerk\\.de/", 10, [], [{ e: 3071 }], [{ v: 3071 }], [{ wait: 500 }, { c: 3071 }], [{ timeout: 1e3, check: "none", wv: 3071 }], {}], [1, "auto_DE_kundenportal.m-net.de_y8l", 0, "^https?://(www\\.)?kundenportal\\.m-net\\.de/", 10, [], [{ e: 3021 }], [{ v: 3021 }], [{ wait: 500 }, { c: 3021 }], [{ timeout: 1e3, check: "none", wv: 3021 }], {}], [1, "auto_DE_kvhb.de_hhf", 0, "^https?://(www\\.)?kvhb\\.de/", 10, [], [{ e: 3072 }], [{ v: 3072 }], [{ wait: 500 }, { c: 3072 }], [{ timeout: 1e3, check: "none", wv: 3072 }], {}], [1, "auto_DE_la.spankbang.com_sva", 0, "^https?://(www\\.)?la\\.spankbang\\.com/", 10, [], [{ e: 3073 }], [{ v: 3073 }], [{ wait: 500 }, { c: 3073 }], [{ timeout: 1e3, check: "none", wv: 3073 }], {}], [1, "auto_DE_lbs.de_6zt", 0, "^https?://(www\\.)?lbs\\.de/", 10, [], [{ e: 3074 }], [{ v: 3074 }], [{ wait: 500 }, { c: 3074 }], [{ timeout: 1e3, check: "none", wv: 3074 }], {}], [1, "auto_DE_lenovo.com_xcv", 0, "^https?://(www\\.)?lenovo\\.com/", 10, [], [{ e: 3075 }], [{ v: 3075 }], [{ wait: 500 }, { c: 3075 }], [{ timeout: 1e3, check: "none", wv: 3075 }], {}], [1, "auto_DE_listando.de_c5i", 0, "^https?://(www\\.)?listando\\.de/", 10, [], [{ e: 3076 }], [{ v: 3076 }], [{ wait: 500 }, { c: 3076 }], [{ timeout: 1e3, check: "none", wv: 3076 }], {}], [1, "auto_DE_lite-magazin.de_e1s", 0, "^https?://(www\\.)?lite-magazin\\.de/", 10, [], [{ e: 3077 }], [{ v: 3077 }], [{ wait: 500 }, { c: 3077 }], [{ timeout: 1e3, check: "none", wv: 3077 }], {}], [1, "auto_DE_m.livejasmin.com_cvg", 0, "^https?://(www\\.)?livejasmin\\.com/", 10, [], [{ e: 3078 }], [{ v: 3078 }], [{ wait: 500 }, { c: 3078 }], [{ timeout: 1e3, check: "none", wv: 3078 }], {}], [1, "auto_DE_mhplus-krankenkasse.de_4xb", 0, "^https?://(www\\.)?mhplus-krankenkasse\\.de/", 10, [], [{ e: 3033 }], [{ v: 3033 }], [{ wait: 500 }, { c: 3033 }], [{ timeout: 1e3, check: "none", wv: 3033 }], {}], [1, "auto_DE_mitarbeiterservice.bayern.de_quh", 0, "^https?://(www\\.)?mitarbeiterservice\\.bayern\\.de/", 10, [], [{ e: 3079 }], [{ v: 3079 }], [{ wait: 500 }, { c: 3079 }], [{ timeout: 1e3, check: "none", wv: 3079 }], {}], [1, "auto_DE_mrmarvis.com_bo5", 0, "^https?://(www\\.)?mrmarvis\\.com/", 10, [], [{ e: 3080 }], [{ v: 3080 }], [{ wait: 500 }, { c: 3080 }], [{ timeout: 1e3, check: "none", wv: 3080 }], {}], [1, "auto_DE_originalteile.mercedes-benz.de_tce", 0, "^https?://(www\\.)?originalteile\\.mercedes-benz\\.de/", 10, [], [{ e: 3081 }], [{ v: 3081 }], [{ wait: 500 }, { c: 3081 }], [{ timeout: 1e3, check: "none", wv: 3081 }], {}], [1, "auto_DE_parqet.com_6wm", 0, "^https?://(www\\.)?parqet\\.com/", 10, [], [{ e: 3082 }], [{ v: 3082 }], [{ wait: 500 }, { c: 3082 }], [{ timeout: 1e3, check: "none", wv: 3082 }], {}], [1, "auto_DE_pflanzenhof-online.de_au2", 0, "^https?://(www\\.)?pflanzenhof-online\\.de/", 10, [], [{ e: 3083 }], [{ v: 3083 }], [{ wait: 500 }, { c: 3083 }], [{ timeout: 1e3, check: "none", wv: 3083 }], {}], [1, "auto_DE_polizei.hessen.de_rsx", 0, "^https?://(www\\.)?polizei\\.hessen\\.de/", 10, [], [{ e: 3084 }], [{ v: 3084 }], [{ wait: 500 }, { c: 3084 }], [{ timeout: 1e3, check: "none", wv: 3084 }], {}], [1, "auto_DE_regierung.oberbayern.bayern.de_zx2_+2", 0, "^https?://(www\\.)?regierung\\.oberbayern\\.bayern\\.de/|^https?://(www\\.)?statistik\\.bayern\\.de/|^https?://(www\\.)?stmb\\.bayern\\.de/", 10, [], [{ e: 3085 }], [{ v: 3085 }], [{ wait: 500 }, { c: 3085 }], [{ timeout: 1e3, check: "none", wv: 3085 }], {}], [1, "auto_DE_roller.de_pjo", 0, "^https?://(www\\.)?roller\\.de/", 10, [], [{ e: 3086 }], [{ v: 3086 }], [{ wait: 500 }, { c: 3086 }], [{ timeout: 1e3, check: "none", wv: 3086 }], {}], [1, "auto_DE_rundfunkbeitrag.de_g4y", 0, "^https?://(www\\.)?rundfunkbeitrag\\.de/", 10, [], [{ e: 3087 }], [{ v: 3087 }], [{ wait: 500 }, { c: 3087 }], [{ timeout: 1e3, check: "none", wv: 3087 }], {}], [1, "auto_DE_schwabach.de_fjr", 0, "^https?://(www\\.)?schwabach\\.de/", 10, [], [{ e: 3088 }], [{ v: 3088 }], [{ wait: 500 }, { c: 3088 }], [{ timeout: 1e3, check: "none", wv: 3088 }], {}], [1, "auto_DE_schwaebisch-hall.de_0g1", 0, "^https?://(www\\.)?schwaebisch-hall\\.de/", 10, [], [{ e: 3089 }], [{ v: 3089 }], [{ wait: 500 }, { c: 3089 }], [{ timeout: 1e3, check: "none", wv: 3089 }], {}], [1, "auto_DE_sellercentral.amazon.de_xi0", 0, "^https?://(www\\.)?sellercentral\\.amazon\\.de/", 10, [], [{ e: 3090 }], [{ v: 3090 }], [{ wait: 500 }, { c: 3090 }], [{ timeout: 1e3, check: "none", wv: 3090 }], {}], [1, "auto_DE_sephora.de_exg", 0, "^https?://(www\\.)?sephora\\.de/", 10, [], [{ e: 3014 }], [{ v: 3014 }], [{ wait: 500 }, { c: 3014 }], [{ timeout: 1e3, check: "none", wv: 3014 }], {}], [1, "auto_DE_solarspeicher24.de_w5k", 0, "^https?://(www\\.)?solarspeicher24\\.de/", 10, [], [{ e: 3091 }], [{ v: 3091 }], [{ wait: 500 }, { c: 3091 }], [{ timeout: 1e3, check: "none", wv: 3091 }], {}], [1, "auto_DE_speedtest.vodafone.de_dha", 0, "^https?://(www\\.)?speedtest\\.vodafone\\.de/", 10, [], [{ e: 3092 }], [{ v: 3092 }], [{ wait: 500 }, { c: 3092 }], [{ timeout: 1e3, check: "none", wv: 3092 }], {}], [1, "auto_DE_steeltoyz.de_i51", 0, "^https?://(www\\.)?steeltoyz\\.de/", 10, [], [{ e: 3093 }], [{ v: 3093 }], [{ wait: 500 }, { c: 3093 }], [{ timeout: 1e3, check: "none", wv: 3093 }], {}], [1, "auto_DE_survival-kompass.de_kv6", 0, "^https?://(www\\.)?survival-kompass\\.de/", 10, [], [{ e: 3094 }], [{ v: 3094 }], [{ wait: 500 }, { c: 3094 }], [{ timeout: 1e3, check: "none", wv: 3094 }], {}], [1, "auto_DE_typografie.info_mnj", 0, "^https?://(www\\.)?typografie\\.info/", 10, [], [{ e: 3095 }], [{ v: 3095 }], [{ wait: 500 }, { c: 3095 }], [{ timeout: 1e3, check: "none", wv: 3095 }], {}], [1, "auto_DE_uni-hildesheim.de_7kj", 0, "^https?://(www\\.)?uni-hildesheim\\.de/", 10, [], [{ e: 3096 }], [{ v: 3096 }], [{ wait: 500 }, { c: 3096 }], [{ timeout: 1e3, check: "none", wv: 3096 }], {}], [1, "auto_DE_uni-mannheim.de_omi", 0, "^https?://(www\\.)?uni-mannheim\\.de/", 10, [], [{ e: 3097 }], [{ v: 3097 }], [{ wait: 500 }, { c: 3097 }], [{ timeout: 1e3, check: "none", wv: 3097 }], {}], [1, "auto_DE_variete.de_6cc", 0, "^https?://(www\\.)?variete\\.de/", 10, [], [{ e: 3098 }], [{ v: 3098 }], [{ wait: 500 }, { c: 3098 }], [{ timeout: 1e3, check: "none", wv: 3098 }], {}], [1, "auto_DE_wien.gv.at_mm2", 0, "^https?://(www\\.)?wien\\.gv\\.at/", 10, [], [{ e: 3099 }], [{ v: 3099 }], [{ wait: 500 }, { c: 3099 }], [{ timeout: 1e3, check: "none", wv: 3099 }], {}], [1, "auto_DE_wolt.com_jyq", 0, "^https?://(www\\.)?wolt\\.com/", 10, [], [{ e: 3100 }], [{ v: 3100 }], [{ wait: 500 }, { c: 3100 }], [{ timeout: 1e3, check: "none", wv: 3100 }], {}], [1, "auto_FR_3ds.com_pa7", 0, "^https?://(www\\.)?3ds\\.com/", 10, [], [{ e: 3101 }], [{ v: 3101 }], [{ wait: 500 }, { c: 3101 }], [{ timeout: 1e3, check: "none", wv: 3101 }], {}], [1, "auto_FR_aefinfo.fr_6r7", 0, "^https?://(www\\.)?aefinfo\\.fr/", 10, [], [{ e: 3102 }], [{ v: 3102 }], [{ wait: 500 }, { c: 3102 }], [{ timeout: 1e3, check: "none", wv: 3102 }], {}], [1, "auto_FR_alinea.com_d9k", 0, "^https?://(www\\.)?alinea\\.com/", 10, [], [{ e: 3103 }], [{ v: 3103 }], [{ wait: 500 }, { c: 3103 }], [{ timeout: 1e3, check: "none", wv: 3103 }], {}], [1, "auto_FR_alinea.com_gst", 0, "^https?://(www\\.)?alinea\\.com/", 10, [], [{ e: 3104 }], [{ v: 3104 }], [{ wait: 500 }, { c: 3104 }], [{ timeout: 1e3, check: "none", wv: 3104 }], {}], [1, "auto_FR_annuaire-inverse-france.com_4oi", 0, "^https?://(www\\.)?annuaire-inverse-france\\.com/", 10, [], [{ e: 3105 }], [{ v: 3105 }], [{ wait: 500 }, { c: 3105 }], [{ timeout: 1e3, check: "none", wv: 3105 }], {}], [1, "auto_FR_asp.gouv.fr_ytt", 0, "^https?://(www\\.)?asp\\.gouv\\.fr/", 10, [], [{ e: 3106 }], [{ v: 3106 }], [{ wait: 500 }, { c: 3106 }], [{ timeout: 1e3, check: "none", wv: 3106 }], {}], [1, "auto_FR_bd-adultes.com_nn2", 0, "^https?://(www\\.)?bd-adultes\\.com/", 10, [], [{ e: 3107 }], [{ v: 3107 }], [{ wait: 500 }, { c: 3107 }], [{ timeout: 1e3, check: "none", wv: 3107 }], {}], [1, "auto_FR_bobvoyeur.com_qm5", 0, "^https?://(www\\.)?bobvoyeur\\.com/", 10, [], [{ e: 3108 }], [{ v: 3108 }], [{ wait: 500 }, { c: 3108 }], [{ timeout: 1e3, check: "none", wv: 3108 }], {}], [1, "auto_FR_bpi.fr_l52", 0, "^https?://(www\\.)?bpi\\.fr/", 10, [], [{ e: 2965 }], [{ v: 2965 }], [{ wait: 500 }, { c: 2965 }], [{ timeout: 1e3, check: "none", wv: 2965 }], {}], [1, "auto_FR_chamrousse.com_i7u", 0, "^https?://(www\\.)?chamrousse\\.com/", 10, [], [{ e: 3109 }], [{ v: 3109 }], [{ wait: 500 }, { c: 3109 }], [{ timeout: 1e3, check: "none", wv: 3109 }], {}], [1, "auto_FR_charliehebdo.fr_smr", 0, "^https?://(www\\.)?charliehebdo\\.fr/", 10, [], [{ e: 3110 }], [{ v: 3110 }], [{ wait: 500 }, { c: 3110 }], [{ timeout: 1e3, check: "none", wv: 3110 }], {}], [1, "auto_FR_cite-sciences.fr_kcx", 0, "^https?://(www\\.)?cite-sciences\\.fr/", 10, [], [{ e: 3111 }], [{ v: 3111 }], [{ wait: 500 }, { c: 3111 }], [{ timeout: 1e3, check: "none", wv: 3111 }], {}], [1, "auto_FR_coe.int_cfo", 0, "^https?://(www\\.)?coe\\.int/", 10, [], [{ e: 3112 }], [{ v: 3112 }], [{ wait: 500 }, { c: 3112 }], [{ timeout: 1e3, check: "none", wv: 3112 }], {}], [1, "auto_FR_coutellerie-tourangelle.com_rcf", 0, "^https?://(www\\.)?coutellerie-tourangelle\\.com/", 10, [], [{ e: 3113 }], [{ v: 3113 }], [{ wait: 500 }, { c: 3113 }], [{ timeout: 1e3, check: "none", wv: 3113 }], {}], [1, "auto_FR_edumoov.com_sij", 0, "^https?://(www\\.)?edumoov\\.com/", 10, [], [{ e: 3114 }], [{ v: 3114 }], [{ wait: 500 }, { c: 3114 }], [{ timeout: 1e3, check: "none", wv: 3114 }], {}], [1, "auto_FR_engie-homeservices.fr_lyo", 0, "^https?://(www\\.)?engie-homeservices\\.fr/", 10, [], [{ e: 3115 }], [{ v: 3115 }], [{ wait: 500 }, { c: 3115 }], [{ timeout: 1e3, check: "none", wv: 3115 }], {}], [1, "auto_FR_es.xhamster.com_f69", 0, "^https?://(www\\.)?es\\.xhamster\\.com/", 10, [], [{ e: 3116 }], [{ v: 3116 }], [{ wait: 500 }, { c: 3116 }], [{ timeout: 1e3, check: "none", wv: 3116 }], {}], [1, "auto_FR_euro-expos.com_tok", 0, "^https?://(www\\.)?euro-expos\\.com/", 10, [], [{ e: 3117 }], [{ v: 3117 }], [{ wait: 500 }, { c: 3117 }], [{ timeout: 1e3, check: "none", wv: 3117 }], {}], [1, "auto_FR_fr.accio.com_zdn", 0, "^https?://(www\\.)?fr\\.accio\\.com/", 10, [], [{ e: 3028 }], [{ v: 3028 }], [{ wait: 500 }, { c: 3028 }], [{ timeout: 1e3, check: "none", wv: 3028 }], {}], [1, "auto_FR_fr.xgimi.com_fzb_+1", 0, "^https?://(www\\.)?fr\\.xgimi\\.com/|^https?://(www\\.)?leminor\\.fr/", 10, [], [{ e: 2946 }], [{ v: 2946 }], [{ wait: 500 }, { c: 2946 }], [{ timeout: 1e3, check: "none", wv: 2946 }], {}], [1, "auto_FR_glamuse.com_n32", 0, "^https?://(www\\.)?glamuse\\.com/", 10, [], [{ e: 3118 }], [{ v: 3118 }], [{ wait: 500 }, { c: 3118 }], [{ timeout: 1e3, check: "none", wv: 3118 }], {}], [1, "auto_FR_gmf.fr_pxt", 0, "^https?://(www\\.)?gmf\\.fr/", 10, [], [{ e: 3119 }], [{ v: 3119 }], [{ wait: 500 }, { c: 3119 }], [{ timeout: 1e3, check: "none", wv: 3119 }], {}], [1, "auto_FR_gov.br_n2f", 0, "^https?://(www\\.)?gov\\.br/", 10, [], [{ e: 3120 }], [{ v: 3120 }], [{ wait: 500 }, { c: 3120 }], [{ timeout: 1e3, check: "none", wv: 3120 }], {}], [1, "auto_FR_greengo.voyage_fg3", 0, "^https?://(www\\.)?greengo\\.voyage/", 10, [], [{ e: 3121 }], [{ v: 3121 }], [{ wait: 500 }, { c: 3121 }], [{ timeout: 1e3, check: "none", wv: 3121 }], {}], [1, "auto_FR_haproxy.com_arh", 0, "^https?://(www\\.)?haproxy\\.com/", 10, [], [{ e: 3122 }], [{ v: 3122 }], [{ wait: 500 }, { c: 3122 }], [{ timeout: 1e3, check: "none", wv: 3122 }], {}], [1, "auto_FR_interencheres.com_c67", 0, "^https?://(www\\.)?interencheres\\.com/", 10, [], [{ e: 3117 }], [{ v: 3117 }], [{ wait: 500 }, { c: 3117 }], [{ timeout: 1e3, check: "none", wv: 3117 }], {}], [1, "auto_FR_ita.xhamster.com_jhk", 0, "^https?://(www\\.)?ita\\.xhamster\\.com/", 10, [], [{ e: 3116 }], [{ v: 3116 }], [{ wait: 500 }, { c: 3116 }], [{ timeout: 1e3, check: "none", wv: 3116 }], {}], [1, "auto_FR_kobo.com_ajz", 0, "^https?://(www\\.)?kobo\\.com/", 10, [], [{ e: 3123 }], [{ v: 3123 }], [{ wait: 500 }, { c: 3123 }], [{ timeout: 1e3, check: "none", wv: 3123 }], {}], [1, "auto_FR_lalibrairie.com_0lt", 0, "^https?://(www\\.)?lalibrairie\\.com/", 10, [], [{ e: 3124 }], [{ v: 3124 }], [{ wait: 500 }, { c: 3124 }], [{ timeout: 1e3, check: "none", wv: 3124 }], {}], [1, "auto_FR_leclercvoyages.com_2o4", 0, "^https?://(www\\.)?leclercvoyages\\.com/", 10, [], [{ e: 3119 }], [{ v: 3119 }], [{ wait: 500 }, { c: 3119 }], [{ timeout: 1e3, check: "none", wv: 3119 }], {}], [1, "auto_FR_lesprosdelapetiteenfance.fr_bng", 0, "^https?://(www\\.)?lesprosdelapetiteenfance\\.fr/", 10, [], [{ e: 3125 }], [{ v: 3125 }], [{ wait: 500 }, { c: 3125 }], [{ timeout: 1e3, check: "none", wv: 3125 }], {}], [1, "auto_FR_maboussoleaidants.fr_f8f", 0, "^https?://(www\\.)?maboussoleaidants\\.fr/", 10, [], [{ e: 3126 }], [{ v: 3126 }], [{ wait: 500 }, { c: 3126 }], [{ timeout: 1e3, check: "none", wv: 3126 }], {}], [1, "auto_FR_magellan-bio.fr_5xr", 0, "^https?://(www\\.)?magellan-bio\\.fr/", 10, [], [{ e: 3127 }], [{ v: 3127 }], [{ wait: 500 }, { c: 3127 }], [{ timeout: 1e3, check: "none", wv: 3127 }], {}], [1, "auto_FR_mam.paris.fr_vbh", 0, "^https?://(www\\.)?mam\\.paris\\.fr/", 10, [], [{ e: 3125 }], [{ v: 3125 }], [{ wait: 500 }, { c: 3125 }], [{ timeout: 1e3, check: "none", wv: 3125 }], {}], [1, "auto_FR_manuels.solutions_3gb", 0, "^https?://(www\\.)?manuels\\.solutions/", 10, [], [{ e: 3128 }], [{ v: 3128 }], [{ wait: 500 }, { c: 3128 }], [{ timeout: 1e3, check: "none", wv: 3128 }], {}], [1, "auto_FR_maty.com_2v7", 0, "^https?://(www\\.)?maty\\.com/", 10, [], [{ e: 3021 }], [{ v: 3021 }], [{ wait: 500 }, { c: 3021 }], [{ timeout: 1e3, check: "none", wv: 3021 }], {}], [1, "auto_FR_mawaqit.net_0cw", 0, "^https?://(www\\.)?mawaqit\\.net/", 10, [], [{ e: 3129 }], [{ v: 3129 }], [{ wait: 500 }, { c: 3129 }], [{ timeout: 1e3, check: "none", wv: 3129 }], {}], [1, "auto_FR_meformerenregion.fr_64b", 0, "^https?://(www\\.)?meformerenregion\\.fr/", 10, [], [{ e: 3130 }], [{ v: 3130 }], [{ wait: 500 }, { c: 3130 }], [{ timeout: 1e3, check: "none", wv: 3130 }], {}], [1, "auto_FR_mesinfos.fr_gt2", 0, "^https?://(www\\.)?mesinfos\\.fr/", 10, [], [{ e: 3131 }], [{ v: 3131 }], [{ wait: 500 }, { c: 3131 }], [{ timeout: 1e3, check: "none", wv: 3131 }], {}], [1, "auto_FR_norauto.fr_mbi", 0, "^https?://(www\\.)?norauto\\.fr/", 10, [], [{ e: 3021 }], [{ v: 3021 }], [{ wait: 500 }, { c: 3021 }], [{ timeout: 1e3, check: "none", wv: 3021 }], {}], [1, "auto_FR_nouslib.com_1tl", 0, "^https?://(www\\.)?nouslib\\.com/", 10, [], [{ e: 3132 }], [{ v: 3132 }], [{ wait: 500 }, { c: 3132 }], [{ timeout: 1e3, check: "none", wv: 3132 }], {}], [1, "auto_FR_oceane.breizhgo.bzh_g7u", 0, "^https?://(www\\.)?oceane\\.breizhgo\\.bzh/", 10, [], [{ e: 3133 }], [{ v: 3133 }], [{ wait: 500 }, { c: 3133 }], [{ timeout: 1e3, check: "none", wv: 3133 }], {}], [1, "auto_FR_pfg.fr_j2d_+1", 0, "^https?://(www\\.)?pfg\\.fr/|^https?://(www\\.)?pointp\\.fr/", 10, [], [{ e: 3119 }], [{ v: 3119 }], [{ wait: 500 }, { c: 3119 }], [{ timeout: 1e3, check: "none", wv: 3119 }], {}], [1, "auto_FR_picwish.com_l0k", 0, "^https?://(www\\.)?picwish\\.com/", 10, [], [{ e: 3134 }], [{ v: 3134 }], [{ wait: 500 }, { c: 3134 }], [{ timeout: 1e3, check: "none", wv: 3134 }], {}], [1, "auto_FR_platform.openai.com_nyz", 0, "^https?://(www\\.)?platform\\.openai\\.com/", 10, [], [{ e: 3135 }], [{ v: 3135 }], [{ wait: 500 }, { c: 3135 }], [{ timeout: 1e3, check: "none", wv: 3135 }], {}], [1, "auto_FR_pointdevente.parionssport.fdj.fr_9uh", 0, "^https?://(www\\.)?pointdevente\\.parionssport\\.fdj\\.fr/", 10, [], [{ e: 3074 }], [{ v: 3074 }], [{ wait: 500 }, { c: 3074 }], [{ timeout: 1e3, check: "none", wv: 3074 }], {}], [1, "auto_FR_politis.fr_g33", 0, "^https?://(www\\.)?politis\\.fr/", 10, [], [{ e: 3136 }], [{ v: 3136 }], [{ wait: 500 }, { c: 3136 }], [{ timeout: 1e3, check: "none", wv: 3136 }], {}], [1, "auto_FR_pretto.fr_kb5", 0, "^https?://(www\\.)?pretto\\.fr/", 10, [], [{ e: 3137 }], [{ v: 3137 }], [{ wait: 500 }, { c: 3137 }], [{ timeout: 1e3, check: "none", wv: 3137 }], {}], [1, "auto_FR_privateaser.com_uco", 0, "^https?://(www\\.)?privateaser\\.com/", 10, [], [{ e: 3138 }], [{ v: 3138 }], [{ wait: 500 }, { c: 3138 }], [{ timeout: 1e3, check: "none", wv: 3138 }], {}], [1, "auto_FR_pro.inserm.fr_omt", 0, "^https?://(www\\.)?pro\\.inserm\\.fr/", 10, [], [{ e: 2965 }], [{ v: 2965 }], [{ wait: 500 }, { c: 2965 }], [{ timeout: 1e3, check: "none", wv: 2965 }], {}], [1, "auto_FR_proantic.com_oyg", 0, "^https?://(www\\.)?proantic\\.com/", 10, [], [{ e: 3139 }], [{ v: 3139 }], [{ wait: 500 }, { c: 3139 }], [{ timeout: 1e3, check: "none", wv: 3139 }], {}], [1, "auto_FR_revue-histoire.fr_jex", 0, "^https?://(www\\.)?revue-histoire\\.fr/", 10, [], [{ e: 3140 }], [{ v: 3140 }], [{ wait: 500 }, { c: 3140 }], [{ timeout: 1e3, check: "none", wv: 3140 }], {}], [1, "auto_FR_rhinoshield.fr_k5l", 0, "^https?://(www\\.)?rhinoshield\\.fr/", 10, [], [{ e: 2946 }], [{ v: 2946 }], [{ wait: 500 }, { c: 2946 }], [{ timeout: 1e3, check: "none", wv: 2946 }], {}], [1, "auto_FR_sephora.fr_k3l", 0, "^https?://(www\\.)?sephora\\.fr/", 10, [], [{ e: 3014 }], [{ v: 3014 }], [{ wait: 500 }, { c: 3014 }], [{ timeout: 1e3, check: "none", wv: 3014 }], {}], [1, "auto_FR_xhamster.desi_pv1_+1", 0, "^https?://(www\\.)?xhamster\\.desi/|^https?://(www\\.)?xhamster3\\.com/", 10, [], [{ e: 3116 }], [{ v: 3116 }], [{ wait: 500 }, { c: 3116 }], [{ timeout: 1e3, check: "none", wv: 3116 }], {}], [1, "auto_GB_3djake.uk_0", 0, "^https?://(www\\.)?3djake\\.uk/", 10, [], [{ e: 3141 }], [{ v: 3141 }], [{ c: 3141 }], [], {}], [1, "auto_GB_actionfraud.org.uk_92k", 0, "^https?://(www\\.)?actionfraud\\.org\\.uk/", 10, [], [{ e: 3142 }], [{ v: 3142 }], [{ wait: 500 }, { c: 3142 }], [{ timeout: 1e3, check: "none", wv: 3142 }], {}], [1, "auto_GB_ancestry.com_0", 0, "^https?://(www\\.)?ancestry\\.com/", 10, [], [{ e: 3143 }], [{ v: 3143 }], [{ wait: 500 }, { c: 3143 }], [{ timeout: 1e3, check: "none", wv: 3143 }], {}], [1, "auto_GB_arte.tv_0", 0, "^https?://(www\\.)?arte\\.tv/", 10, [], [{ e: 3144 }], [{ v: 3144 }], [{ c: 3144 }], [], {}], [1, "auto_GB_bensnaturalhealth.co.uk_0", 0, "^https?://(www\\.)?bensnaturalhealth\\.co\\.uk/", 10, [], [{ e: 3145 }], [{ v: 3145 }], [{ c: 3145 }], [], {}], [1, "auto_GB_bike24.com_0", 0, "^https?://(www\\.)?bike24\\.com/", 10, [], [{ e: 3146 }], [{ v: 3146 }], [{ c: 3146 }], [], {}], [1, "auto_GB_brazzers.com_0", 0, "^https?://(www\\.)?brazzers\\.com/", 10, [], [{ e: 3147 }], [{ v: 3147 }], [{ c: 3147 }], [], {}], [1, "auto_GB_bricksandlogic.co.uk_o5o", 0, "^https?://(www\\.)?bricksandlogic\\.co\\.uk/", 10, [], [{ e: 3148 }], [{ v: 3148 }], [{ wait: 500 }, { c: 3148 }], [{ timeout: 1e3, check: "none", wv: 3148 }], {}], [1, "auto_GB_brightondome.org_iz9", 0, "^https?://(www\\.)?brightondome\\.org/", 10, [], [{ e: 3149 }], [{ v: 3149 }], [{ wait: 500 }, { c: 3149 }], [{ timeout: 1e3, check: "none", wv: 3149 }], {}], [1, "auto_GB_businessclass.com_0", 0, "^https?://(www\\.)?businessclass\\.com/", 10, [], [{ e: 3150 }], [{ v: 3150 }], [{ c: 3150 }], [], {}], [1, "auto_GB_cardmarket.com_oxh", 0, "^https?://(www\\.)?cardmarket\\.com/", 10, [], [{ e: 3151 }], [{ v: 3151 }], [{ wait: 500 }, { c: 3151 }], [{ timeout: 1e3, check: "none", wv: 3151 }], {}], [1, "auto_GB_catawiki.com_0", 0, "^https?://(www\\.)?catawiki\\.com/", 10, [], [{ e: 3152 }], [{ v: 3152 }], [{ c: 3152 }], [], {}], [1, "auto_GB_charlesclinkard.co.uk_0", 0, "^https?://(www\\.)?charlesclinkard\\.co\\.uk/", 10, [], [{ e: 3153 }], [{ v: 3153 }], [{ wait: 500 }, { c: 3153 }], [{ timeout: 1e3, check: "none", wv: 3153 }], {}], [1, "auto_GB_chilternseeds.co.uk_0", 0, "^https?://(www\\.)?chilternseeds\\.co\\.uk/", 10, [], [{ e: 3154 }], [{ v: 3154 }], [{ wait: 500 }, { c: 3154 }], [{ timeout: 1e3, check: "none", wv: 3154 }], {}], [1, "auto_GB_chrono24.com_0", 0, "^https?://(www\\.)?chrono24\\.com/", 10, [], [{ e: 3155 }], [{ v: 3155 }], [{ c: 3155 }], [], {}], [1, "auto_GB_cpfc.co.uk_0", 0, "^https?://(www\\.)?cpfc\\.co\\.uk/", 10, [], [{ e: 3156 }], [{ v: 3156 }], [{ c: 3156 }], [], {}], [1, "auto_GB_deezer.com_0", 0, "^https?://(www\\.)?deezer\\.com/", 10, [], [{ e: 3157 }], [{ v: 3157 }], [{ c: 3157 }], [], {}], [1, "auto_GB_edinburghcastle.scot_h2e", 0, "^https?://(www\\.)?edinburghcastle\\.scot/", 10, [], [{ e: 3158 }], [{ v: 3158 }], [{ wait: 500 }, { c: 3158 }], [{ timeout: 1e3, check: "none", wv: 3158 }], {}], [1, "auto_GB_europarl.europa.eu_0", 0, "^https?://(www\\.)?europarl\\.europa\\.eu/", 10, [], [{ e: 3159 }], [{ v: 3159 }], [{ c: 3159 }], [], {}], [1, "auto_GB_everysaving.co.uk_38t", 0, "^https?://(www\\.)?everysaving\\.co\\.uk/", 10, [], [{ e: 3160 }], [{ v: 3160 }], [{ c: 3160 }], [], {}], [1, "auto_GB_ewrc-results.com_y5f", 0, "^https?://(www\\.)?ewrc-results\\.com/", 10, [], [{ e: 3161 }], [{ v: 3161 }], [{ wait: 500 }, { c: 3161 }], [{ timeout: 1e3, check: "none", wv: 3161 }], {}], [1, "auto_GB_f6s.com_221", 0, "^https?://(www\\.)?f6s\\.com/", 10, [], [{ e: 3162 }], [{ v: 3162 }], [{ wait: 500 }, { c: 3162 }], [{ timeout: 1e3, check: "none", wv: 3162 }], {}], [1, "auto_GB_faphouse.com_0", 0, "^https?://(www\\.)?faphouse\\.com/", 10, [], [{ e: 3163 }], [{ v: 3163 }], [{ c: 3163 }], [], {}], [1, "auto_GB_farmergracy.co.uk_dl3", 0, "^https?://(www\\.)?farmergracy\\.co\\.uk/", 10, [], [{ e: 3164 }], [{ v: 3164 }], [{ wait: 500 }, { c: 3164 }], [{ timeout: 1e3, check: "none", wv: 3164 }], {}], [1, "auto_GB_fca.org.uk_9p9", 0, "^https?://(www\\.)?fca\\.org\\.uk/", 10, [], [{ e: 3165 }], [{ v: 3165 }], [{ wait: 500 }, { c: 3165 }], [{ timeout: 1e3, check: "none", wv: 3165 }], {}], [1, "auto_GB_forum.affinity.serif.com_0", 0, "^https?://(www\\.)?forum\\.affinity\\.serif\\.com/", 10, [], [{ e: 3166 }], [{ v: 3166 }], [{ c: 3166 }], [], {}], [1, "auto_GB_garden4less.co.uk_0", 0, "^https?://(www\\.)?garden4less\\.co\\.uk/", 10, [], [{ e: 3167 }], [{ v: 3167 }], [{ c: 3167 }], [], {}], [1, "auto_GB_glassesdirect.co.uk_bt9", 0, "^https?://(www\\.)?glassesdirect\\.co\\.uk/", 10, [], [{ e: 3168 }], [{ v: 3168 }], [{ wait: 500 }, { c: 3168 }], [{ timeout: 1e3, check: "none", wv: 3168 }], {}], [1, "auto_GB_handbook.fca.org.uk_0", 0, "^https?://(www\\.)?handbook\\.fca\\.org\\.uk/", 10, [], [{ e: 3169 }], [{ v: 3169 }], [{ wait: 500 }, { c: 3169 }], [{ timeout: 1e3, check: "none", wv: 3169 }], {}], [1, "auto_GB_historicenvironment.scot_0", 0, "^https?://(www\\.)?historicenvironment\\.scot/", 10, [], [{ e: 3170 }], [{ v: 3170 }], [{ c: 3170 }], [], {}], [1, "auto_GB_ionos.co.uk_c0a", 0, "^https?://(www\\.)?ionos\\.co\\.uk/", 10, [], [{ e: 3171 }], [{ v: 3171 }], [{ wait: 500 }, { c: 3171 }], [{ timeout: 1e3, check: "none", wv: 3171 }], {}], [1, "auto_GB_kick.com_0", 0, "^https?://(www\\.)?kick\\.com/", 10, [], [{ e: 3172 }], [{ v: 3172 }], [{ c: 3172 }], [], {}], [1, "auto_GB_kinopoisk.ru_0", 0, "^https?://(www\\.)?kinopoisk\\.ru/", 10, [], [{ e: 3173 }], [{ v: 3173 }], [{ wait: 500 }, { c: 3173 }], [{ timeout: 1e3, check: "none", wv: 3173 }], {}], [1, "auto_GB_kirklees.gov.uk_0", 0, "^https?://(www\\.)?kirklees\\.gov\\.uk/", 10, [], [{ e: 3174 }], [{ v: 3174 }], [{ c: 3174 }], [], {}], [1, "auto_GB_lancaster.ac.uk_0", 0, "^https?://(www\\.)?lancaster\\.ac\\.uk/", 10, [], [{ e: 3175 }], [{ v: 3175 }], [{ c: 3175 }], [], {}], [1, "auto_GB_lustery.com_0", 0, "^https?://(www\\.)?lustery\\.com/", 10, [], [{ e: 3176 }], [{ v: 3176 }], [{ c: 3176 }], [], {}], [1, "auto_GB_m.yandex.com_0_+2", 0, "^https?://(www\\.)?m\\.yandex\\.com/|^https?://(www\\.)?online\\.yandex\\.com/|^https?://(www\\.)?xmlsearch\\.yandex\\.ru/", 10, [], [{ e: 3177 }], [{ v: 3177 }], [{ text: "Allow essential cookies", c: 3177 }], [], {}], [1, "auto_GB_mypharmacy.co.uk_0", 0, "^https?://(www\\.)?mypharmacy\\.co\\.uk/", 10, [], [{ e: 3178 }], [{ v: 3178 }], [{ text: "DENY ALL", c: 3178 }], [], {}], [1, "auto_GB_onestream.co.uk_dpx", 0, "^https?://(www\\.)?onestream\\.co\\.uk/", 10, [], [{ e: 3179 }], [{ v: 3179 }], [{ wait: 500 }, { c: 3179 }], [{ timeout: 1e3, check: "none", wv: 3179 }], {}], [1, "auto_GB_outfox.energy_6ux", 0, "^https?://(www\\.)?outfox\\.energy/", 10, [], [{ e: 3180 }], [{ v: 3180 }], [{ wait: 500 }, { c: 3180 }], [{ timeout: 1e3, check: "none", wv: 3180 }], {}], [1, "auto_GB_plumbingworld.co.uk_vmi", 0, "^https?://(www\\.)?plumbingworld\\.co\\.uk/", 10, [], [{ e: 3181 }], [{ v: 3181 }], [{ wait: 500 }, { c: 3181 }], [{ timeout: 1e3, check: "none", wv: 3181 }], {}], [1, "auto_GB_reading.gov.uk_0", 0, "^https?://(www\\.)?reading\\.gov\\.uk/", 10, [], [{ e: 3182 }], [{ v: 3182 }], [{ wait: 500 }, { c: 3182 }], [{ timeout: 1e3, check: "none", wv: 3182 }], {}], [1, "auto_GB_sso.passport.yandex.ru_0_+4", 0, "^https?://(www\\.)?sso\\.passport\\.yandex\\.ru/|^https?://(www\\.)?translate\\.yandex\\.com/|^https?://(www\\.)?ya\\.ru/|^https?://(www\\.)?yandex\\.com\\.tr/|^https?://(www\\.)?yandex\\.com/", 10, [], [{ e: 3173 }], [{ v: 3173 }], [{ c: 3173 }], [], {}], [1, "auto_GB_stoneacre.co.uk_73c", 0, "^https?://(www\\.)?stoneacre\\.co\\.uk/", 10, [], [{ e: 3183 }], [{ v: 3183 }], [{ wait: 500 }, { c: 3183 }], [{ timeout: 1e3, check: "none", wv: 3183 }], {}], [1, "auto_GB_supremecourt.uk_0", 0, "^https?://(www\\.)?supremecourt\\.uk/", 10, [], [{ e: 3184 }], [{ v: 3184 }], [{ c: 3184 }], [{ timeout: 1e3, check: "none", wv: 3184 }], {}], [1, "auto_GB_thebatteryshop.co.uk_0", 0, "^https?://(www\\.)?thebatteryshop\\.co\\.uk/", 10, [], [{ e: 3185 }], [{ v: 3185 }], [{ text: "Reject All", c: 3185 }], [], {}], [1, "auto_GB_thebushcraftstore.co.uk_0_+1", 0, "^https?://(www\\.)?thebushcraftstore\\.co\\.uk/|^https?://(www\\.)?thewoolfactory\\.co\\.uk/", 10, [], [{ e: 3186 }], [{ v: 3186 }], [{ wait: 500 }, { c: 3186 }], [{ timeout: 1e3, check: "none", wv: 3186 }], {}], [1, "auto_GB_trove.scot_xtg", 0, "^https?://(www\\.)?trove\\.scot/", 10, [], [{ e: 3187 }], [{ v: 3187 }], [{ wait: 500 }, { c: 3187 }], [{ timeout: 1e3, check: "none", wv: 3187 }], {}], [1, "auto_GB_truecaller.com_0", 0, "^https?://(www\\.)?truecaller\\.com/", 10, [], [{ e: 3188 }], [{ v: 3188 }], [{ c: 3188 }], [], {}], [1, "auto_GB_vetuk.co.uk_0", 0, "^https?://(www\\.)?vetuk\\.co\\.uk/", 10, [], [{ e: 3189 }], [{ v: 3189 }], [{ wait: 500 }, { c: 3189 }], [{ timeout: 1e3, check: "none", wv: 3189 }], {}], [1, "auto_GB_virgin.com_0", 0, "^https?://(www\\.)?virgin\\.com/", 10, [], [{ e: 3190 }], [{ v: 3190 }], [{ wait: 500 }, { c: 3190 }], [{ timeout: 1e3, check: "none", wv: 3190 }], {}], [1, "auto_GB_vrhump.com_pb5", 0, "^https?://(www\\.)?vrhump\\.com/", 10, [], [{ e: 3191 }], [{ v: 3191 }], [{ wait: 500 }, { c: 3191 }], [{ timeout: 1e3, check: "none", wv: 3191 }], {}], [1, "auto_GB_weldricks.co.uk_0", 0, "^https?://(www\\.)?weldricks\\.co\\.uk/", 10, [], [{ e: 3192 }], [{ v: 3192 }], [{ wait: 500 }, { c: 3192 }], [{ timeout: 1e3, check: "none", wv: 3192 }], {}], [1, "auto_NL_3cx.com_0mf", 0, "^https?://(www\\.)?3cx\\.com/", 10, [], [{ e: 3193 }], [{ v: 3193 }], [{ wait: 500 }, { c: 3193 }], [{ timeout: 1e3, check: "none", wv: 3193 }], {}], [1, "auto_NL_aegon.nl_y2y", 0, "^https?://(www\\.)?aegon\\.nl/", 10, [], [{ e: 3194 }], [{ v: 3194 }], [{ wait: 500 }, { c: 3194 }], [{ timeout: 1e3, check: "none", wv: 3194 }], {}], [1, "auto_NL_app.chatgirl.nl_uqi", 0, "^https?://(www\\.)?app\\.chatgirl\\.nl/", 10, [], [{ e: 3195 }], [{ v: 3195 }], [{ wait: 500 }, { c: 3195 }], [{ timeout: 1e3, check: "none", wv: 3195 }], {}], [1, "auto_NL_beekman.nl_f9e", 0, "^https?://(www\\.)?beekman\\.nl/", 10, [], [{ e: 3196 }], [{ v: 3196 }], [{ wait: 500 }, { c: 3196 }], [{ timeout: 1e3, check: "none", wv: 3196 }], {}], [1, "auto_NL_berivita.com_q3z", 0, "^https?://(www\\.)?berivita\\.com/", 10, [], [{ e: 3197 }], [{ v: 3197 }], [{ wait: 500 }, { c: 3197 }], [{ timeout: 1e3, check: "none", wv: 3197 }], {}], [1, "auto_NL_bibliotheekaanzet.nl_mh6", 0, "^https?://(www\\.)?bibliotheekaanzet\\.nl/", 10, [], [{ e: 3198 }], [{ v: 3198 }], [{ wait: 500 }, { c: 3198 }], [{ timeout: 1e3, check: "none", wv: 3198 }], {}], [1, "auto_NL_bk.nl_tx4", 0, "^https?://(www\\.)?bk\\.nl/", 10, [], [{ e: 2946 }], [{ v: 2946 }], [{ wait: 500 }, { c: 2946 }], [{ timeout: 1e3, check: "none", wv: 2946 }], {}], [1, "auto_NL_blgwonen.nl_g5c", 0, "^https?://(www\\.)?blgwonen\\.nl/", 10, [], [{ e: 3199 }], [{ v: 3199 }], [{ wait: 500 }, { c: 3199 }], [{ timeout: 1e3, check: "none", wv: 3199 }], {}], [1, "auto_NL_braumarkt.com_wfi", 0, "^https?://(www\\.)?braumarkt\\.com/", 10, [], [{ e: 3081 }], [{ v: 3081 }], [{ wait: 500 }, { c: 3081 }], [{ timeout: 1e3, check: "none", wv: 3081 }], {}], [1, "auto_NL_brillen24.nl_08c", 0, "^https?://(www\\.)?brillen24\\.nl/", 10, [], [{ e: 3200 }], [{ v: 3200 }], [{ wait: 500 }, { c: 3200 }], [{ timeout: 1e3, check: "none", wv: 3200 }], {}], [1, "auto_NL_chasse.nl_gs1", 0, "^https?://(www\\.)?chasse\\.nl/", 10, [], [{ e: 3201 }], [{ v: 3201 }], [{ wait: 500 }, { c: 3201 }], [{ timeout: 1e3, check: "none", wv: 3201 }], {}], [1, "auto_NL_chillplanet.nl_xcp", 0, "^https?://(www\\.)?chillplanet\\.nl/", 10, [], [{ e: 3202 }], [{ v: 3202 }], [{ wait: 500 }, { c: 3202 }], [{ timeout: 1e3, check: "none", wv: 3202 }], {}], [1, "auto_NL_chrono24.nl_dco", 0, "^https?://(www\\.)?chrono24\\.nl/", 10, [], [{ e: 2990 }], [{ v: 2990 }], [{ wait: 500 }, { c: 2990 }], [{ timeout: 1e3, check: "none", wv: 2990 }], {}], [1, "auto_NL_clinicaldiagnostics.nl_pmu", 0, "^https?://(www\\.)?clinicaldiagnostics\\.nl/", 10, [], [{ e: 3203 }], [{ v: 3203 }], [{ wait: 500 }, { c: 3203 }], [{ timeout: 1e3, check: "none", wv: 3203 }], {}], [1, "auto_NL_consumentenbond.nl_53g", 0, "^https?://(www\\.)?consumentenbond\\.nl/", 10, [], [{ e: 3204 }], [{ v: 3204 }], [{ wait: 500 }, { c: 3204 }], [{ timeout: 1e3, check: "none", wv: 3204 }], {}], [1, "auto_NL_denboschregion.nl_4x6", 0, "^https?://(www\\.)?denboschregion\\.nl/", 10, [], [{ e: 3205 }], [{ v: 3205 }], [{ wait: 500 }, { c: 3205 }], [{ timeout: 1e3, check: "none", wv: 3205 }], {}], [1, "auto_NL_denieuwebibliotheek.nl_c1z", 0, "^https?://(www\\.)?denieuwebibliotheek\\.nl/", 10, [], [{ e: 3206 }], [{ v: 3206 }], [{ wait: 500 }, { c: 3206 }], [{ timeout: 1e3, check: "none", wv: 3206 }], {}], [1, "auto_NL_discountoffice.nl_2fb", 0, "^https?://(www\\.)?discountoffice\\.nl/", 10, [], [{ e: 3207 }], [{ v: 3207 }], [{ wait: 500 }, { c: 3207 }], [{ timeout: 1e3, check: "none", wv: 3207 }], {}], [1, "auto_NL_ditjesendatjes.nl_0sa", 0, "^https?://(www\\.)?ditjesendatjes\\.nl/", 10, [], [{ e: 3208 }], [{ v: 3208 }], [{ wait: 500 }, { c: 3208 }], [{ timeout: 1e3, check: "none", wv: 3208 }], {}], [1, "auto_NL_duitslandinstituut.nl_d4q", 0, "^https?://(www\\.)?duitslandinstituut\\.nl/", 10, [], [{ e: 3209 }], [{ v: 3209 }], [{ wait: 500 }, { c: 3209 }], [{ timeout: 1e3, check: "none", wv: 3209 }], {}], [1, "auto_NL_effenaar.nl_nia", 0, "^https?://(www\\.)?effenaar\\.nl/", 10, [], [{ e: 3210 }], [{ v: 3210 }], [{ wait: 500 }, { c: 3210 }], [{ timeout: 1e3, check: "none", wv: 3210 }], {}], [1, "auto_NL_fietsonderdelenoutlet.nl_x8u", 0, "^https?://(www\\.)?fietsonderdelenoutlet\\.nl/", 10, [], [{ e: 3211 }], [{ v: 3211 }], [{ wait: 500 }, { c: 3211 }], [{ timeout: 1e3, check: "none", wv: 3211 }], {}], [1, "auto_NL_followthebeat.nl_mx5", 0, "^https?://(www\\.)?followthebeat\\.nl/", 10, [], [{ e: 3212 }], [{ v: 3212 }], [{ wait: 500 }, { c: 3212 }], [{ timeout: 1e3, check: "none", wv: 3212 }], {}], [1, "auto_NL_frankenergie.nl_9xh", 0, "^https?://(www\\.)?frankenergie\\.nl/", 10, [], [{ e: 3213 }], [{ v: 3213 }], [{ wait: 500 }, { c: 3213 }], [{ timeout: 1e3, check: "none", wv: 3213 }], {}], [1, "auto_NL_gezondheidenwetenschap.be_zgi", 0, "^https?://(www\\.)?gezondheidenwetenschap\\.be/", 10, [], [{ e: 3214 }], [{ v: 3214 }], [{ wait: 500 }, { c: 3214 }], [{ timeout: 1e3, check: "none", wv: 3214 }], {}], [1, "auto_NL_independer.nl_ind", 0, "^https?://(www\\.)?independer\\.nl/", 10, [3215], [{ e: 3216 }], [{ v: 3216 }], [{ c: 3216 }], [{ check: "none", timeout: 2e3, wv: 3215 }], {}], [1, "auto_NL_info.mumc.nl_8s5", 0, "^https?://(www\\.)?info\\.mumc\\.nl/", 10, [], [{ e: 3217 }], [{ v: 3217 }], [{ wait: 500 }, { c: 3217 }], [{ timeout: 1e3, check: "none", wv: 3217 }], {}], [1, "auto_NL_inshared.nl_p70", 0, "^https?://(www\\.)?inshared\\.nl/", 10, [], [{ e: 3218 }], [{ v: 3218 }], [{ wait: 500 }, { c: 3218 }], [{ timeout: 1e3, check: "none", wv: 3218 }], {}], [1, "auto_NL_isvw.nl_7x0", 0, "^https?://(www\\.)?isvw\\.nl/", 10, [], [{ e: 3219 }], [{ v: 3219 }], [{ wait: 500 }, { c: 3219 }], [{ timeout: 1e3, check: "none", wv: 3219 }], {}], [1, "auto_NL_kaartje2go.nl_ecw", 0, "^https?://(www\\.)?kaartje2go\\.nl/", 10, [], [{ e: 3220 }], [{ v: 3220 }], [{ wait: 500 }, { c: 3220 }], [{ timeout: 1e3, check: "none", wv: 3220 }], {}], [1, "auto_NL_kathmandu.nl_7j4", 0, "^https?://(www\\.)?kathmandu\\.nl/", 10, [], [{ e: 3221 }], [{ v: 3221 }], [{ wait: 500 }, { c: 3221 }], [{ timeout: 1e3, check: "none", wv: 3221 }], {}], [1, "auto_NL_kvk.nl_0wn", 0, "^https?://(www\\.)?kvk\\.nl/", 10, [], [{ e: 3222 }], [{ v: 3222 }], [{ wait: 500 }, { c: 3222 }], [{ timeout: 1e3, check: "none", wv: 3222 }], {}], [1, "auto_NL_labplusarts.nl_feg", 0, "^https?://(www\\.)?labplusarts\\.nl/", 10, [], [{ e: 3223 }], [{ v: 3223 }], [{ wait: 500 }, { c: 3223 }], [{ timeout: 1e3, check: "none", wv: 3223 }], {}], [1, "auto_NL_lakenhal.nl_3go", 0, "^https?://(www\\.)?lakenhal\\.nl/", 10, [], [{ e: 3224 }], [{ v: 3224 }], [{ wait: 500 }, { c: 3224 }], [{ timeout: 1e3, check: "none", wv: 3224 }], {}], [1, "auto_NL_magazines-motivatie.nl_6o1", 0, "^https?://(www\\.)?magazines-motivatie\\.nl/", 10, [], [{ e: 3225 }], [{ v: 3225 }], [{ wait: 500 }, { c: 3225 }], [{ timeout: 1e3, check: "none", wv: 3225 }], {}], [1, "auto_NL_makro.nl_ror", 0, "^https?://(www\\.)?makro\\.nl/", 10, [], [{ e: 3226 }], [{ v: 3226 }], [{ wait: 500 }, { c: 3226 }], [{ timeout: 1e3, check: "none", wv: 3226 }], {}], [1, "auto_NL_manufactum.nl_w9l", 0, "^https?://(www\\.)?manufactum\\.nl/", 10, [], [{ e: 3010 }], [{ v: 3010 }], [{ wait: 500 }, { c: 3010 }], [{ timeout: 1e3, check: "none", wv: 3010 }], {}], [1, "auto_NL_matrassencheck.nl_cap", 0, "^https?://(www\\.)?matrassencheck\\.nl/", 10, [], [{ e: 3227 }], [{ v: 3227 }], [{ wait: 500 }, { c: 3227 }], [{ timeout: 1e3, check: "none", wv: 3227 }], {}], [1, "auto_NL_mijn.simyo.nl_xm9", 0, "^https?://(www\\.)?mijn\\.simyo\\.nl/", 10, [], [{ e: 3228 }], [{ v: 3228 }], [{ wait: 500 }, { c: 3228 }], [{ timeout: 1e3, check: "none", wv: 3228 }], {}], [1, "auto_NL_mijngelderland.nl_rkj", 0, "^https?://(www\\.)?mijngelderland\\.nl/", 10, [], [{ e: 3229 }], [{ v: 3229 }], [{ wait: 500 }, { c: 3229 }], [{ timeout: 1e3, check: "none", wv: 3229 }], {}], [1, "auto_NL_milieucentraal.nl_p66", 0, "^https?://(www\\.)?milieucentraal\\.nl/", 10, [], [{ e: 3230 }], [{ v: 3230 }], [{ wait: 500 }, { c: 3230 }], [{ timeout: 1e3, check: "none", wv: 3230 }], {}], [1, "auto_NL_muziekgebouw.nl_cob", 0, "^https?://(www\\.)?muziekgebouw\\.nl/", 10, [], [{ e: 3201 }], [{ v: 3201 }], [{ wait: 500 }, { c: 3201 }], [{ timeout: 1e3, check: "none", wv: 3201 }], {}], [1, "auto_NL_nec-nijmegen.nl_04y", 0, "^https?://(www\\.)?nec-nijmegen\\.nl/", 10, [], [{ e: 3231 }], [{ v: 3231 }], [{ wait: 500 }, { c: 3231 }], [{ timeout: 1e3, check: "none", wv: 3231 }], {}], [1, "auto_NL_nl.spankbang.com_bnh_+1", 0, "^https?://(www\\.)?nl\\.spankbang\\.com/|^https?://(www\\.)?spankbang\\.com/", 10, [], [{ e: 3232 }], [{ v: 3232 }], [{ wait: 500 }, { c: 3232 }], [{ timeout: 1e3, check: "none", wv: 3232 }], {}], [1, "auto_NL_orpheus.nl_cxu", 0, "^https?://(www\\.)?orpheus\\.nl/", 10, [], [{ e: 3201 }], [{ v: 3201 }], [{ wait: 500 }, { c: 3201 }], [{ timeout: 1e3, check: "none", wv: 3201 }], {}], [1, "auto_NL_partnerplatform.bol.com_k9h", 0, "^https?://(www\\.)?partnerplatform\\.bol\\.com/", 10, [], [{ e: 3233 }], [{ v: 3233 }], [{ wait: 500 }, { c: 3233 }], [{ timeout: 1e3, check: "none", wv: 3233 }], {}], [1, "auto_NL_persportaal.anp.nl_o32", 0, "^https?://(www\\.)?persportaal\\.anp\\.nl/", 10, [], [{ e: 3234 }], [{ v: 3234 }], [{ wait: 500 }, { c: 3234 }], [{ timeout: 1e3, check: "none", wv: 3234 }], {}], [1, "auto_NL_planteenolijfboom.nl_pdo", 0, "^https?://(www\\.)?planteenolijfboom\\.nl/", 10, [], [{ e: 3235 }], [{ v: 3235 }], [{ wait: 500 }, { c: 3235 }], [{ timeout: 1e3, check: "none", wv: 3235 }], {}], [1, "auto_NL_psv.nl_2pt", 0, "^https?://(www\\.)?psv\\.nl/", 10, [], [{ e: 3236 }], [{ v: 3236 }], [{ wait: 500 }, { c: 3236 }], [{ timeout: 1e3, check: "none", wv: 3236 }], {}], [1, "auto_NL_sanitairkamer.nl_vig", 0, "^https?://(www\\.)?sanitairkamer\\.nl/", 10, [], [{ e: 3237 }], [{ v: 3237 }], [{ wait: 500 }, { c: 3237 }], [{ timeout: 1e3, check: "none", wv: 3237 }], {}], [1, "auto_NL_schaapcitroen.nl_6v0", 0, "^https?://(www\\.)?schaapcitroen\\.nl/", 10, [], [{ e: 3238 }], [{ v: 3238 }], [{ wait: 500 }, { c: 3238 }], [{ timeout: 1e3, check: "none", wv: 3238 }], {}], [1, "auto_NL_scouting.nl_ue4", 0, "^https?://(www\\.)?scouting\\.nl/", 10, [], [{ e: 3239 }], [{ v: 3239 }], [{ wait: 500 }, { c: 3239 }], [{ timeout: 1e3, check: "none", wv: 3239 }], {}], [1, "auto_NL_slachtofferhulp.nl_4m4", 0, "^https?://(www\\.)?slachtofferhulp\\.nl/", 10, [], [{ e: 3240 }], [{ v: 3240 }], [{ wait: 500 }, { c: 3240 }], [{ timeout: 1e3, check: "none", wv: 3240 }], {}], [1, "auto_NL_sprinklr.co_3ww", 0, "^https?://(www\\.)?sprinklr\\.co/", 10, [], [{ e: 2946 }], [{ v: 2946 }], [{ wait: 500 }, { c: 2946 }], [{ timeout: 1e3, check: "none", wv: 2946 }], {}], [1, "auto_NL_stadsschouwburg-utrecht.nl_tcz", 0, "^https?://(www\\.)?stadsschouwburg-utrecht\\.nl/", 10, [], [{ e: 3241 }], [{ v: 3241 }], [{ wait: 500 }, { c: 3241 }], [{ timeout: 1e3, check: "none", wv: 3241 }], {}], [1, "auto_NL_stedelijkmuseumschiedam.nl_y6j", 0, "^https?://(www\\.)?stedelijkmuseumschiedam\\.nl/", 10, [], [{ e: 3142 }], [{ v: 3142 }], [{ wait: 500 }, { c: 3142 }], [{ timeout: 1e3, check: "none", wv: 3142 }], {}], [1, "auto_NL_texelsecourant.nl_v4b", 0, "^https?://(www\\.)?texelsecourant\\.nl/", 10, [], [{ e: 3242 }], [{ v: 3242 }], [{ wait: 500 }, { c: 3242 }], [{ timeout: 1e3, check: "none", wv: 3242 }], {}], [1, "auto_NL_ticketswap.com_d3j", 0, "^https?://(www\\.)?ticketswap\\.com/", 10, [], [{ e: 3243 }], [{ v: 3243 }], [{ wait: 500 }, { c: 3243 }], [{ timeout: 1e3, check: "none", wv: 3243 }], {}], [1, "auto_NL_uit.inapeldoorn.nl_gt5", 0, "^https?://(www\\.)?uit\\.inapeldoorn\\.nl/", 10, [], [{ e: 3244 }], [{ v: 3244 }], [{ wait: 500 }, { c: 3244 }], [{ timeout: 1e3, check: "none", wv: 3244 }], {}], [1, "auto_NL_vlaanderen.be_c8w", 0, "^https?://(www\\.)?vlaanderen\\.be/", 10, [], [{ e: 3245 }], [{ v: 3245 }], [{ wait: 500 }, { c: 3245 }], [{ timeout: 1e3, check: "none", wv: 3245 }], {}], [1, "auto_NL_vvaa.nl_avs", 0, "^https?://(www\\.)?vvaa\\.nl/", 10, [], [{ e: 3246 }], [{ v: 3246 }], [{ wait: 500 }, { c: 3246 }], [{ timeout: 1e3, check: "none", wv: 3246 }], {}], [1, "auto_NL_werkenbijdefensie.nl_0cv", 0, "^https?://(www\\.)?werkenbijdefensie\\.nl/", 10, [], [{ e: 3247 }], [{ v: 3247 }], [{ wait: 500 }, { c: 3247 }], [{ timeout: 1e3, check: "none", wv: 3247 }], {}], [1, "auto_NL_winkelstraat.nl_hxj", 0, "^https?://(www\\.)?winkelstraat\\.nl/", 10, [], [{ e: 3248 }], [{ v: 3248 }], [{ wait: 500 }, { c: 3248 }], [{ timeout: 1e3, check: "none", wv: 3248 }], {}], [1, "auto_NL_woonnetrijnmond.nl_itj", 0, "^https?://(www\\.)?woonnetrijnmond\\.nl/", 10, [], [{ e: 3249 }], [{ v: 3249 }], [{ wait: 500 }, { c: 3249 }], [{ timeout: 1e3, check: "none", wv: 3249 }], {}], [1, "auto_NL_zuidas.nl_abm", 0, "^https?://(www\\.)?zuidas\\.nl/", 10, [], [{ e: 3250 }], [{ v: 3250 }], [{ wait: 500 }, { c: 3250 }], [{ timeout: 1e3, check: "none", wv: 3250 }], {}], [1, "auto_NO_apxml.com_l6n", 0, "^https?://(www\\.)?apxml\\.com/", 10, [], [{ e: 3251 }], [{ v: 3251 }], [{ wait: 500 }, { c: 3251 }], [{ timeout: 1e3, check: "none", wv: 3251 }], {}], [1, "auto_NO_arbetsformedlingen.se_wxz", 0, "^https?://(www\\.)?arbetsformedlingen\\.se/", 10, [], [{ e: 3252 }], [{ v: 3252 }], [{ wait: 500 }, { c: 3252 }], [{ timeout: 1e3, check: "none", wv: 3252 }], {}], [1, "auto_NO_avanza.se_b5b", 0, "^https?://(www\\.)?avanza\\.se/", 10, [], [{ e: 3253 }], [{ v: 3253 }], [{ wait: 500 }, { c: 3253 }], [{ timeout: 1e3, check: "none", wv: 3253 }], {}], [1, "auto_NO_avidafinance.com_jwc", 0, "^https?://(www\\.)?avidafinance\\.com/", 10, [], [{ e: 3254 }], [{ v: 3254 }], [{ wait: 500 }, { c: 3254 }], [{ timeout: 1e3, check: "none", wv: 3254 }], {}], [1, "auto_NO_beducated.com_jjt", 0, "^https?://(www\\.)?beducated\\.com/", 10, [], [{ e: 3255 }], [{ v: 3255 }], [{ wait: 500 }, { c: 3255 }], [{ timeout: 1e3, check: "none", wv: 3255 }], {}], [1, "auto_NO_dsa.no_w2b", 0, "^https?://(www\\.)?dsa\\.no/", 10, [], [{ e: 3256 }], [{ v: 3256 }], [{ wait: 500 }, { c: 3256 }], [{ timeout: 1e3, check: "none", wv: 3256 }], {}], [1, "auto_NO_goteborg.com_q7v", 0, "^https?://(www\\.)?goteborg\\.com/", 10, [], [{ e: 3257 }], [{ v: 3257 }], [{ wait: 500 }, { c: 3257 }], [{ timeout: 1e3, check: "none", wv: 3257 }], {}], [1, "auto_NO_hacksmith.store_49t", 0, "^https?://(www\\.)?hacksmith\\.store/", 10, [], [{ e: 3258 }], [{ v: 3258 }], [{ wait: 500 }, { c: 3258 }], [{ timeout: 1e3, check: "none", wv: 3258 }], {}], [1, "auto_NO_hotelfinse1222.no_zpm", 0, "^https?://(www\\.)?hotelfinse1222\\.no/", 10, [], [{ e: 3235 }], [{ v: 3235 }], [{ wait: 500 }, { c: 3235 }], [{ timeout: 1e3, check: "none", wv: 3235 }], {}], [1, "auto_NO_jaksta.com_own", 0, "^https?://(www\\.)?jaksta\\.com/", 10, [], [{ e: 3259 }], [{ v: 3259 }], [{ wait: 500 }, { c: 3259 }], [{ timeout: 1e3, check: "none", wv: 3259 }], {}], [1, "auto_NO_ledigajobb.se_tx3", 0, "^https?://(www\\.)?ledigajobb\\.se/", 10, [], [{ e: 3260 }], [{ v: 3260 }], [{ wait: 500 }, { c: 3260 }], [{ timeout: 1e3, check: "none", wv: 3260 }], {}], [1, "auto_NO_saseurobonusmastercard.se_oqd", 0, "^https?://(www\\.)?saseurobonusmastercard\\.se/", 10, [], [{ e: 3261 }], [{ v: 3261 }], [{ wait: 500 }, { c: 3261 }], [{ timeout: 1e3, check: "none", wv: 3261 }], {}], [1, "auto_NO_sverigesradio.se_a00", 0, "^https?://(www\\.)?sverigesradio\\.se/", 10, [], [{ e: 3262 }], [{ v: 3262 }], [{ wait: 500 }, { c: 3262 }], [{ timeout: 1e3, check: "none", wv: 3262 }], {}], [1, "auto_NO_swedbank.se_63w", 0, "^https?://(www\\.)?swedbank\\.se/", 10, [], [{ e: 3263 }], [{ v: 3263 }], [{ wait: 500 }, { c: 3263 }], [{ timeout: 1e3, check: "none", wv: 3263 }], {}], [1, "auto_NO_tingstad.com_z23", 0, "^https?://(www\\.)?tingstad\\.com/", 10, [], [{ e: 3264 }], [{ v: 3264 }], [{ wait: 500 }, { c: 3264 }], [{ timeout: 1e3, check: "none", wv: 3264 }], {}], [1, "auto_NO_truecaller.com_sdg", 0, "^https?://(www\\.)?truecaller\\.com/", 10, [], [{ e: 3265 }], [{ v: 3265 }], [{ wait: 500 }, { c: 3265 }], [{ timeout: 1e3, check: "none", wv: 3265 }], {}], [1, "auto_NO_webank.it_6rx", 0, "^https?://(www\\.)?webank\\.it/", 10, [], [{ e: 3266 }], [{ v: 3266 }], [{ wait: 500 }, { c: 3266 }], [{ timeout: 1e3, check: "none", wv: 3266 }], {}], [1, "auto_NO_xrealgirl.com_r42", 0, "^https?://(www\\.)?xrealgirl\\.com/", 10, [], [{ e: 3267 }], [{ v: 3267 }], [{ wait: 500 }, { c: 3267 }], [{ timeout: 1e3, check: "none", wv: 3267 }], {}], [1, "auto_US_amazon.jobs_0", 0, "^https?://(www\\.)?amazon\\.jobs/", 10, [], [{ e: 3268 }], [{ v: 3268 }], [{ wait: 500 }, { c: 3268 }], [{ timeout: 1e3, check: "none", wv: 3268 }], {}], [1, "auto_US_amsoil.com_fgr", 0, "^https?://(www\\.)?amsoil\\.com/", 10, [], [{ e: 3269 }], [{ v: 3269 }], [{ c: 3269 }], [], {}], [1, "auto_US_balsamhill.com_oxq", 0, "^https?://(www\\.)?balsamhill\\.com/", 10, [], [{ e: 3270 }], [{ v: 3270 }], [{ wait: 500 }, { c: 3270 }], [{ timeout: 1e3, check: "none", wv: 3270 }], {}], [1, "auto_US_computerworld.com_0_+1", 0, "^https?://(www\\.)?computerworld\\.com/|^https?://(www\\.)?csoonline\\.com/", 10, [], [{ e: 3271 }], [{ v: 3271 }], [{ text: "Do not accept", c: 3271 }], [], {}], [1, "auto_US_coupons.slickdeals.net_wya", 0, "^https?://(www\\.)?coupons\\.slickdeals\\.net/", 10, [], [{ e: 3272 }], [{ v: 3272 }], [{ wait: 500 }, { c: 3272 }], [{ timeout: 1e3, check: "none", wv: 3272 }], {}], [1, "auto_US_deezer.com_0", 0, "^https?://(www\\.)?deezer\\.com/", 10, [], [{ e: 3273 }], [{ v: 3273 }], [{ text: "Refuse", c: 3273 }], [], {}], [1, "auto_US_emagine-entertainment.com_0", 0, "^https?://(www\\.)?emagine-entertainment\\.com/", 10, [], [{ e: 3274 }], [{ v: 3274 }], [{ wait: 500 }, { c: 3274 }], [{ timeout: 1e3, check: "none", wv: 3274 }], {}], [1, "auto_US_fawesome.tv_gjx", 0, "^https?://(www\\.)?fawesome\\.tv/", 10, [], [{ e: 3275 }], [{ v: 3275 }], [{ wait: 500 }, { c: 3275 }], [{ timeout: 1e3, check: "none", wv: 3275 }], {}], [1, "auto_US_forum.affinity.serif.com_0", 0, "^https?://(www\\.)?forum\\.affinity\\.serif\\.com/", 10, [], [{ e: 3276 }], [{ v: 3276 }], [{ text: "\xA0Reject Cookies", c: 3276 }], [], {}], [1, "auto_US_forum.prusa3d.com_0", 0, "^https?://(www\\.)?forum\\.prusa3d\\.com/", 10, [], [{ e: 3277 }], [{ v: 3277 }], [{ text: "Reject All", c: 3277 }], [], {}], [1, "auto_US_greenpan.us_cko", 0, "^https?://(www\\.)?greenpan\\.us/", 10, [], [{ e: 2946 }], [{ v: 2946 }], [{ wait: 500 }, { c: 2946 }], [{ timeout: 1e3, check: "none", wv: 2946 }], {}], [1, "auto_US_ligonier.org_0", 0, "^https?://(www\\.)?ligonier\\.org/", 10, [], [{ e: 3278 }], [{ v: 3278 }], [{ text: "Strictly Necessary", c: 3278 }], [], {}], [1, "auto_US_lilly.com_lh6_+1", 0, "^https?://(www\\.)?lilly\\.com/|^https?://(www\\.)?zepbound\\.lilly\\.com/", 10, [], [{ e: 3279 }], [{ v: 3279 }], [{ wait: 500 }, { c: 3279 }], [{ timeout: 1e3, check: "none", wv: 3279 }], {}], [1, "auto_US_mixedbread.ai_tge", 0, "^https?://(www\\.)?mixedbread\\.com/", 10, [], [{ e: 3251 }], [{ v: 3251 }], [{ wait: 500 }, { c: 3251 }], [{ timeout: 1e3, check: "none", wv: 3251 }], {}], [1, "auto_US_musicnotes.com_0", 0, "^https?://(www\\.)?musicnotes\\.com/", 10, [], [{ exists: ["#polaris-css-lockdown-container", ".polaris-consent-widget"] }], [{ visible: ["#polaris-css-lockdown-container", ".polaris-consent-widget"] }], [{ if: { exists: ["#polaris-css-lockdown-container", "[data-testid='oneClickOptOutLink']"] }, then: [{ waitForThenClick: ["#polaris-css-lockdown-container", "[data-testid='oneClickOptOutLink']"] }], else: [{ hide: ["#polaris-css-lockdown-container", ".polaris-consent-widget"] }] }], [], {}], [1, "auto_US_newmedicare.com_6jp", 0, "^https?://(www\\.)?newmedicare\\.com/", 10, [], [{ e: 3280 }], [{ v: 3280 }], [{ wait: 500 }, { c: 3280 }], [{ timeout: 1e3, check: "none", wv: 3280 }], {}], [1, "auto_US_peptidesciences.com_0", 0, "^https?://(www\\.)?peptidesciences\\.com/", 10, [], [{ e: 3281 }], [{ v: 3281 }], [{ wait: 500 }, { c: 3281 }], [{ timeout: 1e3, check: "none", wv: 3281 }], {}], [1, "auto_US_semrush.com_0", 0, "^https?://(www\\.)?semrush\\.com/", 10, [], [{ e: 3282 }], [{ v: 3282 }], [{ wait: 500 }, { c: 3282 }], [{ timeout: 1e3, check: "none", wv: 3282 }], {}], [1, "auto_US_sso.passport.yandex.ru_0_+5", 0, "^https?://(www\\.)?sso\\.passport\\.yandex\\.ru/|^https?://(www\\.)?translate\\.yandex\\.com/|^https?://(www\\.)?tv\\.yandex\\.com/|^https?://(www\\.)?ya\\.ru/|^https?://(www\\.)?yandex\\.com\\.tr/|^https?://(www\\.)?yandex\\.com/", 10, [], [{ e: 3283 }], [{ v: 3283 }], [{ text: "Allow essential cookies", c: 3283 }], [], {}], [1, "auto_US_truecaller.com_0", 0, "^https?://(www\\.)?truecaller\\.com/", 10, [], [{ e: 3284 }], [{ v: 3284 }], [{ text: "Accept Necessary Cookies", c: 3284 }], [], {}], [1, "auto_US_weingartz.com_aso", 0, "^https?://(www\\.)?weingartz\\.com/", 10, [], [{ e: 3285 }], [{ v: 3285 }], [{ c: 3285 }], [], {}], [1, "autohero", 2, "^https?://(www\\.)?autohero\\.com/", 22, [], [{ e: 3286 }], [{ v: 3286 }], [{ c: 3286 }, { wv: 3287 }, { if: { e: 3288 }, then: [{ all: true, k: 3288 }] }, { c: 3286 }], [{ cc: 3289 }], {}], [1, "autokostencheck.de", 0, "^https?://(www\\.)?autokostencheck\\.de/", 10, [3290, 3291], [{ e: 3292 }], [{ any: [{ check: "any", v: 3293 }, { visible: ["#cmpwrapper", "#cmpbox .cmpmore"], check: "any" }] }], [{ waitForVisible: ["#cmpwrapper", "#cmpbox .cmpmore"], timeout: 2e3, check: "any", optional: true }, { if: { visible: ["#cmpwrapper", "#cmpbox .cmpmore"], check: "any" }, then: [{ eval: "EVAL_CONSENTMANAGER_3" }] }, { timeout: 2e3, check: "any", optional: true, wv: 3293 }, { if: { check: "any", v: 3293 }, then: [{ k: 3293 }, { c: 3294 }] }], [{ eval: "EVAL_CONSENTMANAGER_5" }, { eval: "EVAL_COOKIEFIRST_0" }], {}], [1, "axesor", 2, "^https?://([\\w-]+\\.)?axesor\\.es/", 22, [3295], [{ e: 3296 }], [{ v: 3296 }], [{ c: 3296 }], [{ cc: 3297 }], {}], [1, "aytm", 2, "^https?://(www\\.|)?aytm\\.com/", 22, [550], [{ e: 3298 }], [{ v: 3298 }], [{ c: 3298 }], [], {}], [1, "b1ng-cc", 2, "^https?://(www\\.)?(giorni-festivi\\.it|feiertag\\.co|dia-festivo\\.es)/", 22, [3299], [{ e: 3300 }], [{ v: 3300 }], [{ c: 3300 }, { timeout: 5e3, optional: true, w: 3301 }, { if: { e: 3301 }, then: [{ optional: true, k: 1171 }, { all: true, optional: true, k: 1173 }, { optional: true, k: 1174 }] }], [{ cc: 3302 }], {}], [1, "babylist", 2, "^https?://(www\\.)?babylist\\.com/", 22, [3303, 3304], [{ e: 3303 }], [{ v: 3305 }], [{ c: 3305 }, { timeout: 2e3, optional: true, c: 3306 }], [], {}], [1, "babylonbee", 2, "^https?://(www\\.)?babylonbee\\.com/", 22, [], [{ e: 3307 }], [{ v: 3307 }], [{ retry: 3, c: 3307 }], [{ check: "none", v: 3307 }], {}], [1, "bahn-de", 0, "^https://([\\w-]+\\.)?bahn\\.de/", 10, [], [{ exists: ["body > div:first-child", "#consent-layer"] }], [{ visible: ["body > div:first-child", "#consent-layer"] }], [{ waitForThenClick: ["body > div:first-child", "#consent-layer .js-accept-essential-cookies"] }], [{ eval: "EVAL_BAHN_TEST" }], { intermediate: false }], [1, "bakeca", 2, "^https?://([\\w-]+\\.)?bakecaincontrii\\.com/", 22, [], [{ e: 3308 }], [{ v: 3309 }], [{ c: 3310 }], [{ check: "none", timeout: 2e3, wv: 3309 }], {}], [1, "bandcamp.com", 2, "^https://([a-z0-9-]+\\.)?bandcamp\\.com", 22, [], [{ e: 3311 }], [{ any: [{ v: 3312 }, { visible: ["menu-bar", ".cookie-dialog-initial"] }, { visible: ["page-footer", ".cookie-dialog-initial"] }] }], [{ any: [{ timeout: 500, c: 3313 }, { waitForThenClick: ["menu-bar", ".cookie-dialog-initial button.g-button.outline"], timeout: 500 }, { waitForThenClick: ["page-footer", ".cookie-dialog-initial button.g-button.outline"], timeout: 500 }] }], [{ wait: 500 }, { cc: 3314 }], {}], [1, "bankmillennium.pl", 2, "^https?://(www\\.)?bankmillennium\\.pl/", 22, [3315], [{ e: 3315 }], [{ v: 3315 }], [{ waitForThenClick: ["#cookie_modal_placeholder dialog.bm-modal--cookie", "xpath///button[contains(normalize-space(.), 'Odrzu')]"] }], [{ cc: 3316 }, { cc: 3317 }], {}], [1, "banknorwegian", 2, "^https?://(www\\.)?banknorwegian\\.(no|se|dk|fi|de|es)/", 22, [3318], [{ e: 3319 }], [{ v: 3320 }], [{ c: 3320 }, { wv: 3321 }, { all: true, optional: true, k: 3322 }, { k: 3321 }, { check: "none", timeout: 3e3, optional: true, wv: 3321 }], [{ check: "none", v: 3323 }], {}], [1, "batteriexperten.com", 2, "^https?://(\\w+\\.)?(batteriexperten\\.com|teknikdelar\\.se|teknikkdeler\\.no|teknikdele\\.dk|tekniikkaosat\\.fi)/", 22, [3324], [{ e: 3325 }], [{ v: 3325 }], [{ if: { e: 3326 }, then: [{ c: 3327 }], else: [{ c: 3328 }, { wv: 3329 }, { all: true, optional: true, k: 3330 }, { c: 3331 }] }, { check: "none", wv: 3325 }], [{ cc: 3332 }], {}], [1, "bbc.com", 2, "^https://(www\\.)?bbc\\.(com|co\\.uk)/", 22, [3333, 3334], [{ any: [{ e: 3335 }, { e: 3336 }] }], [{ any: [{ v: 3335 }, { v: 3336 }] }], [{ if: { e: 3336 }, then: [{ c: 3336 }], else: [{ c: 3337 }] }], [{ wait: 500 }, { cc: 3338 }], {}], [1, "bbc.com-orb-cosmetic", 1, "^https://(www\\.)?bbc\\.(com|co\\.uk)/", 22, [3339], [{ e: 3340 }], [{ v: 3340 }], [{ h: 3339 }], [{ check: "none", v: 3341 }], {}], [1, "bcferries.com", 2, "^https?://(\\w+\\.)?bcferries\\.com/", 22, [3342], [{ e: 3343 }], [{ v: 3342 }], [{ c: 3344 }, { c: 3345 }, { c: 3346 }], [], {}], [1, "berelax.com", 2, "^https://(www\\.)?berelax\\.com/", 22, [3347], [{ e: 3348 }], [{ v: 3349 }], [{ c: 3350 }, { check: "none", timeout: 2e3, wv: 3349 }], [], {}], [1, "bgk.pl", 2, "^https?://(www\\.)?bgk\\.pl/", 22, [3351], [{ e: 3352 }], [{ v: 3353 }], [{ retry: 3, c: 3354 }, { wv: 3352 }, { all: true, optional: true, k: 3355 }, { c: 3356 }], [{ cc: 3357 }], {}], [1, "biblia-online.pl", 1, "^https?://(\\w+\\.)?biblia-online\\.pl/", 10, [3358], [{ e: 3359 }], [{ v: 3358 }], [{ h: 3358 }], [{ check: "none", v: 3358 }], {}], [1, "bibliotheek-nl", 0, "^https?://(?:www\\.)?(?:onlinebibliotheek|bibliotheek)\\.nl/", 10, [3360], [{ e: 3361 }], [{ v: 3360 }], [{ c: 3362 }], [], {}], [1, "bing.com-notice", 2, "^https?://(\\w+\\.)?bing\\.com/", 22, [3363], [{ e: 3364 }], [{ v: 3363 }], [{ wv: 3365 }, { eval: "EVAL_BING_NOTICE_CLOSE" }], [{ check: "none", v: 3363 }], {}], [2, "bing.com-notice-cosmetic", 1, "^https?://(\\w+\\.)?bing\\.com/", 22, [3366], [{ e: 3366 }], [{ v: 3366 }], [{ h: 3366 }, { addStyle: "top: 0 !important", selector: "header#hdr", optional: true }], [{ check: "none", v: 3366 }], {}], [1, "bitauto", 2, "^https?://(\\w+\\.)?bitauto\\.com/", 22, [3367], [{ e: 3368 }], [{ v: 3368 }], [{ c: 3368 }, { timeout: 4e3, optional: true, wv: 2802 }, { if: { v: 2802 }, then: [{ k: 1171 }, { all: true, optional: true, k: 1173 }, { optional: true, k: 1174 }] }], [{ cc: 3369 }], {}], [1, "blitzrechner", 2, "^https?://(www\\.)?blitzrechner\\.de/", 22, [3370], [{ e: 3370 }], [{ v: 3371 }], [{ all: true, optional: true, k: 3372 }, { c: 3373 }], [{ cc: 3374 }], {}], [1, "bo3.gg", 2, "^https?://([a-z0-9-]+\\.)?bo3\\.gg/", 22, [3375], [{ e: 3375 }], [{ v: 3376 }], [{ c: 3377 }, { wv: 3378 }, { all: true, optional: true, k: 3379 }, { c: 3380 }], [{ cc: 3381 }], {}], [1, "bokadirekt.se", 2, "^https?://(\\w+\\.)?bokadirekt\\.se/", 22, [], [{ e: 3382 }], [{ v: 3382 }], [{ c: 3383 }, { wv: 3384 }, { if: { e: 3385 }, then: [{ all: true, k: 3385 }] }, { k: 3384 }], [{ cc: 3386 }], {}], [1, "bokio", 2, "^https?://(www\\.)?bokio\\.(se|co\\.uk)/", 22, [1006], [{ e: 3387 }], [{ v: 3388 }], [{ c: 3387 }], [{ cc: 3389 }], {}], [1, "brainshare", 2, "^https?://([\\w-]+\\.)?brainshare\\.pl/", 22, [3390], [{ e: 3391 }], [{ v: 3390 }], [{ c: 3391 }], [{ cc: 3392 }], {}], [1, "brandsforstudents.ch", 2, "^https?://(www\\.)?brandsfor(students|employees)\\.ch/", 22, [3393], [{ e: 3394 }], [{ v: 3393 }], [{ c: 3394 }, { w: 3395 }, { all: true, optional: true, k: 3396 }, { c: 3397 }], [{ cc: 3398 }], {}], [1, "breuninger", 2, "^https://(www\\.)?breuninger\\.com/", 22, [3399], [{ e: 3400 }], [{ v: 3401 }], [{ c: 3400 }], [{ cc: 3402 }], {}], [1, "bricklink", 2, "^https?://([a-z0-9-]+\\.)?bricklink\\.com/", 22, [3403], [{ e: 3403 }], [{ v: 3403 }], [{ c: 3404 }], [{ cc: 3405 }], {}], [1, "bringatrailer", 2, "^https?://(www\\.)?bringatrailer\\.com/", 22, [], [{ e: 3406 }, { cc: 3407 }], [{ v: 3408 }], [{ c: 3406 }], [{ cc: 3409 }], {}], [1, "bt-com-au", 2, "^https?://(www\\.)?bt\\.com\\.au/", 22, [3410], [{ e: 3410 }], [{ v: 3411 }], [{ c: 3412 }], [{ check: "none", timeout: 1e3, wv: 3410 }], {}], [1, "buonmedico", 2, "^https?://(www\\.)?buonmedico\\.it/", 22, [3413], [{ e: 3414 }], [{ v: 3414 }], [{ c: 3414 }], [{ cc: 3415 }], {}], [1, "calclife.net", 2, "^https?://(www\\.)?calclife\\.net/", 22, [3416], [{ e: 3417 }], [{ v: 3417 }], [{ c: 3417 }, { wv: 3418 }, { if: { e: 3419 }, then: [{ k: 3420 }] }, { k: 3421 }], [], {}], [1, "calcolo.tools", 2, "^https?://(www\\.)?calcolo\\.tools/", 22, [3422], [{ v: 3423 }], [{ v: 3423 }], [{ c: 3423 }, { timeout: 5e3, optional: true, w: 3301 }, { if: { e: 3301 }, then: [{ optional: true, k: 1171 }, { all: true, optional: true, k: 1173 }, { optional: true, k: 1174 }] }], [{ check: "none", v: 3422 }], {}], [1, "canva", 2, "^https?://(www\\.)?canva\\.com/", 22, [3424], [{ e: 3424 }], [{ v: 3424 }], [{ c: 3425 }, { c: 3426 }, { c: 3427 }], [{ wait: 1e3 }, { cc: 3428 }], {}], [1, "canyon.com", 2, "^https://www\\.canyon\\.com/", 22, [3429], [{ e: 3429 }], [{ v: 3429 }], [{ k: 3430 }, { c: 3431 }], [], {}], [1, "capitalone-ca", 2, "^https://(www\\.)?capitalone\\.ca/", 22, [], [{ e: 3432 }], [{ v: 3432 }], [{ c: 3432 }], [{ cc: 3433 }], {}], [1, "carpetaciudadana", 2, "^https?://(\\w+\\.)?carpetaciudadana\\.gob\\.es/", 22, [3434], [{ e: 3435 }], [{ v: 3436 }], [{ c: 3435 }, { wv: 3437 }, { if: { e: 3438 }, then: [{ k: 3437 }] }, { retry: 2, retryInterval: 600, c: 3439 }], [{ cc: 3440 }], {}], [1, "carre.nl", 2, "^https?://(www\\.)?carre\\.nl/", 10, [3441], [{ e: 3442 }], [{ v: 3441 }], [{ c: 3442 }], [], {}], [1, "casier-judiciaire-justice-gouv-fr", 2, "^https?://casier-judiciaire\\.justice\\.gouv\\.fr/", 22, [3443], [{ e: 3444 }], [{ v: 3443 }], [{ c: 3445 }, { check: "none", timeout: 2e3, optional: true, wv: 3443 }], [{ check: "none", v: 3443 }], {}], [1, "ccm-net", 1, "^https?://([\\w-]+\\.)?ccm\\.net/", 22, [109, 3446], [{ any: [{ e: 110 }, { e: 3446 }] }], [{ any: [{ v: 109 }, { v: 3446 }] }], [{ h: 3446 }, { if: { v: 109 }, then: [{ waitForThenClick: ["iframe[srcdoc*='frame-root']", ".button__skip"] }] }], [], {}], [1, "cdu-edu-au", 1, "^https?://(\\w+\\.)?cdu\\.edu\\.au/", 22, [3447], [{ e: 3448 }], [{ v: 3447 }], [{ h: 3447 }], [], {}], [1, "chairish", 2, "^https?://(www\\.)?chairish\\.com/", 22, [3449], [{ e: 3450 }], [{ v: 3451 }], [{ c: 3450 }, { wv: 3452 }, { all: true, optional: true, k: 3453 }, { c: 3454 }], [{ cc: 3455 }], {}], [1, "channel4.com", 2, "^https?://(www\\.)?channel4\\.com/", 10, [312, 3456], [{ e: 3457 }], [{ v: 3457 }], [{ c: 3458 }], [{ cc: 3459 }], {}], [1, "chatgpt", 0, "^https?://(\\w+\\.)?chatgpt\\.com/", 10, [3460], [{ e: 3461 }], [{ v: 3462 }], [{ c: 3461 }], [{ timeout: 1e3, check: "none", wv: 3463 }], {}], [1, "chemicalbook", 1, "^https?://(\\w+\\.)?chemicalbook\\.com/", 22, [3464], [{ e: 3465 }], [{ v: 3464 }], [{ h: 3464 }], [], {}], [1, "chimnie-co-uk", 2, "^https?://(\\w+\\.)?chimnie\\.co\\.uk/", 22, [3466], [{ e: 3466 }], [{ v: 3466 }], [{ c: 3467 }, { c: 3468 }], [{ check: "none", timeout: 2e3, wv: 3466 }], {}], [1, "chomikuj", 2, "^https?://(\\w+\\.)?chomikuj\\.pl/", 22, [], [{ e: 3469 }], [{ v: 3469 }], [{ c: 3469 }], [{ check: "none", timeout: 2e3, wv: 3470 }], {}], [1, "citizensadvice", 2, "^https?://(\\w+\\.)?citizensadvice\\.org\\.uk/", 22, [3471], [{ e: 3472 }], [{ v: 3472 }], [{ c: 3472 }, { timeout: 2e3, optional: true, c: 3473 }], [{ cc: 858 }], {}], [1, "civilservicejobs-service-gov-uk", 2, "^https?://(\\w+\\.)?civilservicejobs\\.service\\.gov\\.uk/", 22, [3474], [{ e: 3475 }], [{ v: 3475 }], [{ c: 3475 }, { check: "none", timeout: 2e3, optional: true, wv: 3474 }], [{ cc: 3476 }], {}], [1, "clasohlson.com", 2, "^https?://(www\\.)?clasohlson\\.com/", 22, [3477], [{ e: 3478 }], [{ v: 3479 }], [{ retry: 10, retryInterval: 500, c: 3480 }, { check: "none", wv: 3479 }], [{ cc: 3481 }], {}], [1, "clustrmaps.com", 1, "^https://(www\\.)?clustrmaps\\.com/", 22, [1358], [{ e: 1358 }], [{ v: 1358 }], [{ h: 1358 }], [], {}], [1, "cnmc.gob.es", 2, "^https?://(\\w+\\.)?cnmc\\.gob\\.es/", 22, [3482], [{ e: 3482 }], [{ v: 3483 }, { e: 3484 }], [{ c: 3485 }, { timeout: 2e3, check: "none", wv: 3483 }], [], {}], [3, "community.screwfix.com", 2, "^https?://community\\.screwfix\\.com/", 10, [3486], [{ e: 3487 }], [{ v: 3487 }], [{ c: 3488 }, { stylesheet: "#consent-banner::before { display: none !important; }", stylesheetId: "screwfix-backdrop" }], [{ cc: 3489 }], {}], [1, "comparandonos", 2, "^https?://(www\\.)?comparandonos\\.com/", 10, [47], [{ e: 3490 }], [{ v: 47 }], [{ c: 3490 }, { wv: 3491 }, { if: { e: 3492 }, then: [{ k: 3493 }] }, { if: { e: 3494 }, then: [{ k: 3495 }] }, { k: 3491 }], [{ timeout: 1e3, check: "none", wv: 3496 }, { check: "none", v: 47 }], {}], [1, "concordia-ch", 2, "^https://(\\w+\\.)?concordia\\.(ch|li)/", 22, [3497], [{ e: 3497 }], [{ v: 3498 }], [{ c: 3499 }], [{ cc: 3500 }], {}], [1, "conjointly", 2, "^https://(www\\.)?conjointly\\.com/", 22, [3501], [{ e: 3502 }], [{ v: 3502 }], [{ if: { e: 3503 }, then: [{ k: 3504 }] }, { c: 3505 }], [{ cc: 3506 }], {}], [1, "consilium-europa-eu", 2, "^https://(www\\.)?consilium\\.europa\\.eu/", 22, [], [{ e: 3507 }], [{ visible: ["gsc-cookie-banner", "dialog[data-consent-banner]"] }], [{ waitForThenClick: ["gsc-cookie-banner", ".gsc-cookie-banner__actions gsc-button:first-child"] }], [{ waitForVisible: ["gsc-cookie-banner", "dialog[data-consent-banner]"], check: "none", timeout: 2e3 }], {}], [1, "controlgame", 2, "^https?://(www\\.)?controlgame\\.com/", 22, [], [{ e: 3508 }], [{ v: 3508 }], [{ retry: 10, retryInterval: 500, c: 3508 }], [{ cc: 846 }], {}], [1, "copaamerica", 1, "^https://(www\\.)?copaamerica\\.com/", 22, [3509], [{ e: 3509 }], [{ v: 3509 }], [{ h: 3509 }], [], {}], [1, "copilot-microsoft", 0, "^https://copilot\\.microsoft\\.com/", 10, [47], [{ e: 47 }], [{ v: 47 }], [{ c: 3510 }], [{ cc: 3511 }], {}], [1, "costco", 1, "^https?://([\\w-]+\\.)*costco(businesscentre|businessdelivery)?\\.(ca|com)/", 22, [3512], [{ e: 3513 }], [{ v: 3512 }], [{ h: 3514 }], [], {}], [1, "creditunionsa", 1, "^https?://(www\\.)?creditunionsa\\.com\\.au/", 22, [3515], [{ e: 3516 }], [{ v: 3515 }], [{ h: 3515 }], [], {}], [1, "crtm", 2, "^https?://(\\w+\\.)?crtm\\.es/", 22, [47, 3517], [{ e: 3518 }], [{ v: 47 }], [{ c: 3518 }, { c: 3519 }], [{ cc: 3520 }], {}], [1, "cstudios", 1, "^https?://(\\w+\\.)?cstudios\\.pl/", 22, [3521], [{ e: 3522 }], [{ v: 3522 }], [{ h: 3521 }], [{ check: "none", v: 3521 }], {}], [1, "csu-landtag-de", 2, "^https://(www\\.|)?csu-landtag\\.de", 22, [3523], [{ e: 3523 }], [{ v: 3523 }], [{ k: 3524 }], [], {}], [1, "ctv-disneyonice", 2, "^https://www\\.ctv\\.co\\.jp/disneyonice(?:/|[?#]|$)", 22, [3525], [{ e: 3526 }], [{ v: 3526 }], [{ c: 3527 }], [{ cc: 3528 }], {}], [1, "cuvva.com", 0, "^https://(www\\.)?cuvva\\.com/", 22, [], [{ e: 3529 }], [{ v: 3529 }, { negated: true, cc: 3530 }], [{ retry: 10, retryInterval: 500, c: 3531 }, { wait: 500 }, { if: { v: 3531 }, then: [{ retry: 10, retryInterval: 500, c: 3531 }] }], [{ cc: 3532 }], {}], [1, "dailymotion.com", 2, "^https://(www\\.)?dailymotion\\.com/", 22, [3533], [{ e: 3534 }], [{ v: 3535 }], [{ c: 3536 }], [{ cc: 3537 }], {}], [1, "dalecarnegie", 2, "^https?://(\\w+\\.)?dalecarnegie\\.com/", 22, [], [{ e: 3538 }], [{ v: 3539 }], [{ c: 3538 }, { wv: 3540 }, { all: true, optional: true, k: 3541 }, { k: 3540 }], [{ cc: 3542 }], {}], [1, "darkounity.com", 2, "^https://(www\\.)?darkounity\\.com/", 22, [3543], [{ exists: ["gdpr-consent-popup", "[data-action='necessary-only']"] }], [{ visible: ["gdpr-consent-popup", ".banner"] }], [{ waitForThenClick: ["gdpr-consent-popup", "[data-action='necessary-only']"] }], [{ waitForVisible: ["gdpr-consent-popup", ".banner"], timeout: 2e3, check: "none" }], {}], [1, "datezone", 2, "^https?://(\\w+\\.)?datezone\\.com/", 22, [3544], [{ e: 3545 }], [{ v: 3544 }], [{ c: 3546 }, { w: 3547 }, { all: true, optional: true, k: 3548 }, { c: 3549 }, { check: "none", timeout: 3e3, wv: 3544 }], [{ cc: 3550 }], {}], [1, "deliveroo", 2, "^https?://(\\w+\\.)?deliveroo\\.(co\\.uk|\\w+)/", 22, [3551], [{ e: 3551 }], [{ v: 3551 }], [{ c: 3552 }, { check: "none", wv: 3551 }], [{ cc: 2414 }], {}], [1, "delta.com", 1, "^https://www\\.delta\\.com/", 22, [3553], [{ e: 3554 }], [{ v: 3554 }], [{ h: 3554 }], [], {}], [2, "depop", 1, "^https://(www\\.)?depop\\.com/", 22, [2367, 3555], [{ e: 2367 }], [{ visible: ["#transcend-consent-manager", "#consentManagerMainDialog"] }], [{ h: 2367 }, { h: 3555 }, { timeout: 3e3, optional: true, w: 3555 }, { addStyle: "overflow: auto !important", selector: "body", optional: true }], [], {}], [1, "dhbbank", 2, "^https?://(www\\.)?dhbbank\\.(com|nl|be|de)/", 22, [3556], [{ e: 3557 }], [{ v: 3557 }], [{ c: 3557 }], [{ check: "none", timeout: 3e3, wv: 3556 }, { cc: 3558 }], {}], [1, "diag.pl", 2, "^https?://(\\w+\\.)?diag\\.pl/", 22, [], [{ e: 3559 }], [{ v: 3559 }], [{ c: 3559 }], [{ check: "none", timeout: 3e3, wv: 3559 }], {}], [1, "dijklander", 2, "^https?://(www\\.)?dijklander\\.nl/", 22, [3560], [{ e: 3561 }], [{ v: 3560 }], [{ c: 3561 }], [{ check: "none", timeout: 2e3, wv: 3560 }], {}], [1, "dji", 0, "^https://(\\w+\\.)+dji\\.com/", 22, [3562], [{ e: 3563 }], [{ e: 3562 }], [{ c: 3564 }], [{ cc: 3565 }], {}], [1, "dndbeyond", 2, "^https://(www\\.)?dndbeyond\\.com/", 22, [3566], [{ e: 3566 }], [{ v: 3566 }], [{ c: 3567 }], [{ cc: 3568 }], {}], [1, "doc-du-juriste", 2, "^https?://(www\\.)?doc-du-juriste\\.com/", 22, [3569], [{ e: 3570 }], [{ v: 3570 }], [{ c: 3570 }, { c: 3571 }, { c: 3572 }, { check: "none", optional: true, wv: 1006 }], [{ cc: 3573 }], {}], [1, "doktorabc", 2, "^https?://(\\w+\\.)?doktorabc\\.com/", 22, [3574], [{ e: 3575 }], [{ v: 3575 }], [{ c: 3576 }, { wv: 3577 }, { all: true, optional: true, k: 3578 }, { c: 3579 }], [{ cc: 3580 }], {}], [1, "dola", 2, "^https?://(www\\.)?dola\\.com/", 22, [3581], [{ e: 3582 }], [{ v: 3582 }], [{ c: 3583 }], [{ timeout: 1e3, check: "none", wv: 3581 }], {}], [1, "dolphinfitness", 2, "^https?://(www\\.)?dolphinfitness\\.co\\.uk/", 22, [3584], [{ e: 3585 }], [{ v: 3585 }], [{ c: 3586 }, { wv: 3587 }, { k: 3588 }, { k: 3589 }, { k: 3587 }, { c: 3590 }], [{ e: 3591 }, { e: 3592 }, { e: 3593 }], {}], [1, "dtu-findit", 2, "^https?://findit\\.dtu\\.dk/", 22, [], [{ e: 3594 }], [{ v: 3594 }], [{ if: { e: 3595 }, then: [{ k: 3595 }] }, { c: 3596 }], [{ cc: 3597 }, { negated: true, cc: 3598 }], {}], [1, "dwaalzin.nu", 2, "^https?://(www\\.)?dwaalzin\\.nu/", 22, [531], [{ e: 3599 }], [{ v: 3599 }], [{ c: 3599 }], [{ cc: 1569 }], {}], [1, "eastlink", 2, "^https?://(www\\.)?eastlink\\.ca/", 22, [], [{ e: 3600 }], [{ e: 3601 }, { negated: true, cc: 3602 }], [{ timeout: 1e4, wv: 3603 }, { k: 3600 }, { wv: 3604 }, { if: { e: 3605 }, then: [{ k: 3606 }] }, { if: { e: 3607 }, then: [{ k: 3608 }] }, { k: 3601 }], [], {}], [1, "ebay", 0, "^https://(www\\.)?ebay\\.([.a-z]+)/", 10, [3609], [{ e: 3609 }], [{ v: 3609 }], [{ c: 3610 }, { check: "none", timeout: 2e3, optional: true, wv: 3609 }, { if: { v: 3609 }, then: [{ timeout: 2e3, c: 3610 }] }], [], {}], [1, "ecosia", 2, "^https://www\\.ecosia\\.org/", 22, [3611], [{ e: 3612 }], [{ v: 3612 }], [{ c: 3613 }], [], {}], [1, "edebe", 0, "^https?://plus\\.edebe\\.com/", 22, [1844], [{ e: 3614 }], [{ v: 3614 }], [{ c: 3615 }, { wv: 3616 }, { if: { e: 3617 }, then: [{ k: 3617 }] }, { c: 3618 }], [{ cc: 3619 }], {}], [1, "edpb-edps", 2, "^https://(www\\.)?(edpb|edps)\\.europa\\.eu/", 22, [3620], [{ e: 3621 }], [{ v: 3620 }], [{ c: 3621 }], [{ cc: 3622 }], {}], [1, "eduardolosilla.es", 2, "^https?://(www\\.)?eduardolosilla\\.es/", 22, [3623], [{ e: 3623 }], [{ v: 3623 }], [{ c: 3624 }], [{ check: "none", timeout: 1e3, wv: 3623 }], {}], [1, "educastur-alojaweb", 1, "^https?://alojaweb\\.educastur\\.es/", 22, [3625], [{ e: 3625 }], [{ v: 3625 }], [{ h: 3625 }], [], {}], [1, "ef-ccpa", 0, "^https://(www\\.)?eforms\\.com", 22, [3626], [{ e: 3626 }], [{ v: 3626 }], [{ c: 3627 }], [], {}], [1, "ekd.de", 2, "^https?://(\\w+\\.)?ekd\\.de/", 22, [3628], [{ e: 3629 }], [{ v: 3628 }], [{ c: 3629 }], [{ cc: 3630 }], {}], [1, "elektro-material-ch", 1, "^https?://(www\\.)?elektro-material\\.ch/", 22, [3631], [{ e: 3632 }], [{ v: 3633 }], [{ h: 3631 }], [], {}], [1, "elsevier", 2, "^https?://(\\w+\\.)?elsevier\\.(com|support)/", 22, [3634], [{ e: 3635 }], [{ v: 3635 }], [{ if: { e: 3636 }, then: [{ retry: 3, retryInterval: 1e3, c: 3636 }], else: [{ retry: 3, retryInterval: 1e3, c: 3637 }, { timeout: 1e4, wv: 2410 }, { all: true, optional: true, c: 2412 }, { timeout: 5e3, c: 2413 }] }, { check: "none", timeout: 1e4, wv: 3638 }], [{ cc: 2414 }], {}], [1, "eltax-lta-go-jp", 2, "^https://(?:[^/]+\\.)?eltax\\.lta\\.go\\.jp/", 22, [3639], [{ e: 3640 }], [{ v: 3640 }], [{ c: 3640 }], [], {}], [1, "emr-ch", 2, "^https?://(www\\.)?(emr|emfeedback)\\.ch/", 22, [3641], [{ e: 3642 }], [{ v: 3641 }], [{ all: true, optional: true, k: 3643 }, { retry: 3, retryInterval: 500, c: 3642 }], [{ cc: 3644 }], {}], [1, "endeavorhealth", 2, "^https?://(www\\.)?endeavorhealth\\.org/", 22, [3645], [{ e: 3646 }], [{ v: 3646 }], [{ retry: 2, retryInterval: 1e3, c: 3647 }], [{ check: "none", timeout: 2e3, wv: 3646 }], {}], [1, "endocrine.org", 2, "^https?://(www\\.)?endocrine\\.org/", 22, [], [{ e: 3648 }], [{ v: 3649 }], [{ c: 3649 }], [{ timeout: 1e3, check: "none", wv: 3648 }], {}], [1, "eneba", 0, "^https?://(www\\.)?eneba\\.com/", 10, [3650], [{ e: 3651 }], [{ v: 3651 }], [{ c: 3652 }, { timeout: 2e3, optional: true, wv: 3653 }, { if: { check: "none", v: 3653 }, then: [{ k: 3652 }] }, { wv: 3653 }, { all: true, optional: true, k: 3654 }, { c: 3655 }], [{ wait: 500 }, { cc: 3656 }], {}], [1, "england.nhs.uk", 2, "^https?://(www\\.)?england\\.nhs\\.uk/", 22, [3657], [{ e: 3658 }], [{ v: 3657 }], [{ c: 3658 }, { wv: 475 }, { if: { e: 3659 }, then: [{ all: true, k: 3659 }] }, { c: 477 }], [{ cc: 3660 }], {}], [1, "ensembleartsphilly", 2, "^https?://(\\w+\\.)?ensembleartsphilly\\.org/", 22, [3661], [{ e: 3662 }], [{ v: 3663 }], [{ c: 3662 }], [{ cc: 3664 }], {}], [1, "eon.pl", 2, "^https?://(\\w+\\.)?eon\\.pl/", 22, [3665], [{ e: 3666 }], [{ v: 3665 }], [{ c: 3667 }, { wv: 3668 }, { optional: true, k: 3669 }, { k: 3670 }], [{ cc: 3671 }], {}], [1, "epidemicsound.com", 2, "^https?://(\\w+\\.)?epidemicsound\\.com/", 22, [3672], [{ e: 3673 }], [{ v: 3674 }], [{ w: 2413 }, { c: 3674 }, { wv: 2410 }, { all: true, optional: true, k: 3675 }, { c: 2413 }, { check: "none", wv: 3674 }], [{ cc: 2414 }], {}], [1, "eplaque", 2, "^https?://(www\\.)?eplaque\\.fr/", 22, [3676], [{ e: 3677 }], [{ v: 3676 }], [{ c: 3678 }], [{ cc: 3679 }], {}], [1, "erli-pl", 2, "^https?://(\\w+\\.)?erli\\.pl/", 22, [], [{ e: 3680 }], [{ v: 3680 }], [{ c: 3681 }, { c: 3682 }], [{ check: "none", timeout: 2e3, wv: 3680 }], {}], [1, "eroticiracconti-it", 1, "^https?://(www\\.)?eroticiracconti\\.it/", 22, [3683], [{ e: 3684 }], [{ v: 3685 }], [{ h: 3683 }], [], {}], [1, "ersatzteile-24", 2, "^https?://(www\\.)?ersatzteile-24\\.com/", 22, [3686], [{ e: 3687 }], [{ v: 3688 }], [{ retry: 5, retryInterval: 1e3, c: 3689 }, { wv: 3690 }, { k: 3690 }, { k: 3687 }], [{ cc: 3691 }, { cc: 3692 }], {}], [1, "escaparium.ca", 2, "^https://(www\\.)?escaparium\\.ca/", 22, [], [{ e: 3693 }], [{ v: 3694 }], [{ c: 3695 }, { c: 3696 }], [{ cc: 3697 }], {}], [1, "escorteintime", 2, "^https?://(www\\.)?escorteintime\\.com/", 22, [3698], [{ e: 3698 }], [{ v: 3698 }], [{ c: 3699 }, { c: 3700 }], [{ check: "none", timeout: 2e3, wv: 3698 }], {}], [1, "espace-personnel.agirc-arrco.fr", 2, "^https://espace-personnel\\.agirc-arrco\\.fr/", 22, [3701], [{ e: 3702 }], [{ v: 3702 }], [{ c: 3703 }], [], {}], [1, "europa-eu", 2, "^https://([a-z0-9.-]*\\.)?europa\\.eu/", 22, [3704], [{ e: 3704 }], [{ v: 3704 }], [{ c: 3705 }], [{ cc: 3706 }], {}], [1, "eventfrog", 2, "^https?://(www\\.)?eventfrog\\.(ch|de|at)/", 22, [3707], [{ e: 3708 }], [{ v: 3709 }], [{ wv: 3708 }, { wait: 500 }, { k: 3708 }], [{ cc: 3710 }], {}], [1, "evium.de", 2, "^https://(www\\.)?evium\\.de/", 22, [3711], [{ e: 3711 }], [{ v: 3711 }], [{ c: 3712 }, { check: "none", timeout: 2e3, wv: 3711 }], [], {}], [1, "evozen.fr", 2, "^https?://(\\w+\\.)?evozen\\.fr/", 22, [3713], [{ e: 3714 }], [{ v: 3713 }], [{ c: 3715 }], [], {}], [1, "facebook", 2, "^https://(www\\.)?facebook\\.com/", 22, [], [{ e: 3716 }], [{ v: 3716 }], [{ wait: 500 }, { retry: 3, retryInterval: 500, c: 3716 }, { check: "none", wv: 3717 }], [], {}], [1, "facebook-mobile", 2, "^https://(m|mbasic)\\.facebook\\.com/", 22, [1645], [{ e: 1645 }], [{ v: 1645 }], [{ c: 3718 }, { check: "none", wv: 1645 }], [], {}], [1, "fascicolosanitario-lombardia", 0, "^https?://(\\w+\\.)?fascicolosanitario\\.regione\\.lombardia\\.it/", 22, [617], [{ e: 3719 }], [{ v: 3720 }], [{ c: 3721 }], [{ cc: 3722 }], {}], [1, "fellow.pl", 2, "^https?://(\\w+\\.)?(fellow|fll)\\.pl/", 22, [], [{ e: 3723 }], [{ v: 3723 }], [{ c: 3724 }], [], {}], [1, "fepblue.org", 2, "^https?://(www\\.)?fepblue\\.org/", 22, [3725], [{ e: 3726 }], [{ v: 3727 }], [{ c: 3726 }], [{ check: "none", timeout: 2e3, wv: 3726 }], {}], [1, "fideuram", 2, "^https?://(www\\.)?fideuram\\.it/", 22, [3728], [{ e: 3729 }], [{ v: 3729 }], [{ c: 3729 }], [{ cc: 3730 }], {}], [1, "financestrategists.com", 1, "^https://(www\\.)?financestrategists\\.com/", 22, [3731], [{ e: 3731 }], [{ v: 3731 }], [{ h: 3731 }], [], {}], [1, "fis-data-exchange", 2, "^https://services\\.dataexchange\\.fiscloudservices\\.com/", 22, [3732], [{ e: 3733 }], [{ v: 3733 }], [{ c: 3733 }], [{ cc: 3734 }], {}], [1, "florisvanbommel", 2, "^https?://(\\w+\\.)?florisvanbommel\\.com/", 22, [3735], [{ e: 3736 }], [{ v: 3736 }], [{ c: 3736 }], [{ cc: 3737 }], {}], [1, "fn-se", 2, "^https?://(www\\.)?fn\\.se/", 22, [1990], [{ e: 3738 }], [{ v: 3739 }], [{ c: 3740 }, { wv: 3741 }, { k: 3741 }], [{ cc: 3742 }], {}], [1, "fod-fujitv", 2, "^https?://fod\\.fujitv\\.co\\.jp/", 22, [3743], [{ v: 3744 }], [{ v: 3743 }], [{ c: 3744 }], [{ cc: 3745 }], {}], [1, "fondoest.it", 2, "^https?://(www\\.)?fondoest\\.it/", 22, [3746], [{ e: 3747 }], [{ v: 3748 }], [{ c: 3748 }, { c: 3749 }], [{ cc: 3750 }], {}], [1, "fonts.google.com", 2, "^https?://fonts\\.google\\.com/", 22, [3751], [{ e: 3752 }], [{ v: 3751 }], [{ c: 3753 }], [{ check: "none", timeout: 2e3, wv: 3751 }], {}], [1, "forsakringskassan.se", 2, "^https?://(\\w+\\.)?forsakringskassan\\.se/", 22, [3754], [{ e: 3755 }], [{ v: 3755 }], [{ c: 3756 }], [{ cc: 3757 }], {}], [1, "foto-erhardt", 2, "^https?://(www\\.)?foto-erhardt\\.(de|com|fr)/", 22, [], [{ e: 3758 }], [{ v: 3759 }], [{ c: 3758 }, { c: 3760 }], [{ cc: 3761 }], {}], [1, "fpl-com", 2, "^https?://(www\\.)?(fpl|fplhome)\\.com/", 22, [3762], [{ e: 3762 }], [{ v: 3762 }], [{ c: 3763 }], [{ check: "none", v: 3762 }], {}], [1, "fuqer", 2, "^https?://(\\w+\\.)?fuqer\\.com/", 22, [3764], [{ e: 3765 }], [{ v: 3765 }], [{ retry: 10, retryInterval: 500, c: 3765 }], [{ cc: 3766 }], {}], [1, "futoncompany", 2, "^https?://(\\w+\\.)?futoncompany\\.co\\.uk/", 22, [3767], [{ e: 3767 }], [{ v: 3767 }], [{ c: 3768 }, { wv: 3769 }, { all: true, optional: true, k: 3770 }, { k: 3771 }], [{ wait: 1500 }, { cc: 3772 }], {}], [1, "futurabolzano.it", 1, "^https?://(www\\.)?futurabolzano\\.it/", 22, [3773], [{ e: 3773 }], [{ v: 3773 }], [{ h: 3773 }], [{ check: "none", v: 3773 }], {}], [1, "gamefound-com", 1, "^https://(www\\.)?gamefound\\.com/", 22, [3774], [{ e: 3775 }], [{ v: 3776 }], [{ h: 3774 }], [], {}], [1, "gays-cruising", 2, "^https?://([\\w-]+\\.)?gays-cruising\\.com/", 22, [3777], [{ e: 3778 }], [{ v: 3777 }], [{ c: 3779 }, { wv: 3780 }, { all: true, optional: true, k: 3781 }, { k: 3782 }], [{ cc: 3783 }], {}], [1, "geeks-for-geeks", 1, "^https://www\\.geeksforgeeks\\.org/", 22, [3416], [{ e: 3416 }], [{ v: 3416 }], [{ h: 3416 }], [], {}], [1, "geni.com", 1, "^https://(www\\.)?geni\\.com/", 22, [3784], [{ e: 3784 }], [{ v: 3784 }], [{ h: 3784 }], [], {}], [1, "ggzstandaarden.nl", 2, "^https?://(www\\.)?ggzstandaarden\\.nl/", 22, [1319], [{ e: 3785 }], [{ v: 3785 }], [{ c: 3785 }], [{ cc: 3786 }], {}], [1, "givedirectly", 2, "^https?://(www\\.)?givedirectly\\.org/", 22, [3787], [{ e: 3787 }], [{ v: 3787 }], [{ retry: 3, c: 3788 }], [{ check: "none", wv: 3787 }], {}], [1, "gla.ac.uk", 2, "^https?://(\\w+\\.)?gla\\.ac\\.uk/", 22, [3789], [{ e: 3790 }], [{ v: 3790 }], [{ c: 3791 }, { k: 3792 }, { k: 3793 }], [{ cc: 3794 }, { cc: 3795 }, { cc: 3796 }], {}], [1, "glastonburyfestivals", 0, "^https://(\\w+\\.)?glastonburyfestivals\\.co\\.uk/", 22, [3797], [{ e: 3798 }], [{ v: 3798 }], [{ c: 3799 }], [], {}], [1, "gleeden", 2, "^https?://(\\w+\\.)?gleeden\\.com/", 22, [3800], [{ e: 3801 }], [{ v: 3802 }], [{ c: 3803 }, { check: "none", wv: 3802 }], [{ check: "none", v: 3802 }], {}], [1, "gnoccaforum", 2, "^https?://(www\\.)?gnoccaforum\\.com/", 22, [1844], [{ e: 3804 }], [{ v: 1844 }], [{ c: 3805 }], [{ check: "none", timeout: 2e3, wv: 1844 }], {}], [1, "gobiernodecanarias", 2, "^https?://(\\w+\\.)?(gobiernodecanarias\\.org|gobcan\\.net)/", 22, [3806], [{ e: 3807 }], [{ v: 3806 }], [{ c: 3808 }, { wv: 3807 }, { optional: true, k: 3809 }, { c: 3810 }], [{ cc: 3811 }], {}], [1, "gov-pl-cookies-info", 1, "^https?://([\\w-]+\\.)*gov\\.pl/", 22, [3812], [{ e: 3813 }], [{ v: 3814 }], [{ h: 3812 }], [], {}], [1, "greatschools", 2, "^https?://(www\\.)?greatschools\\.org/", 22, [3815], [{ e: 3816 }], [{ v: 3815 }], [{ c: 3816 }], [{ cc: 3817 }], {}], [1, "groundnews", 0, "^https://(www\\.)?ground\\.news/", 22, [], [{ e: 3818 }], [{ v: 3818 }], [{ waitForThenClick: [".fixed:has([data-testid=closeCookieBanner])", "xpath///button[contains(., 'Manage cookies')]"] }, { waitFor: ["[data-testid=modal]", "xpath///span[contains(., 'Essential cookies')]"] }, { click: ["[data-testid=modal]", "xpath///button[contains(., 'On')]"], all: true, optional: true }, { waitForThenClick: ["[data-testid=modal]", "xpath///button[contains(., 'Save & reload')]"] }], [], {}], [1, "groupama.fr", 2, "^https?://(www\\.)?groupama\\.fr/", 22, [3819], [{ e: 3820 }], [{ v: 3819 }], [{ timeout: 1e4, c: 3821 }], [{ cc: 3822 }], {}], [1, "gsmpunt.nl", 2, "^https?://(\\w+\\.)?gsmpunt\\.nl/", 22, [3823], [{ e: 3824 }], [{ v: 3825 }], [{ c: 3824 }, { wv: 3826 }, { all: true, optional: true, k: 3827 }, { k: 3826 }], [{ cc: 3828 }], {}], [1, "gundam-base-entry", 2, "^https://(www\\.)?gundam-base-entry\\.net/", 22, [3829], [{ e: 3830 }], [{ v: 3830 }], [{ c: 3830 }, { c: 3831 }, { check: "none", wv: 3829 }], [{ cc: 3832 }], {}], [1, "hamradio.com", 1, "^https?://(www\\.)?hamradio\\.com/", 22, [3456], [{ e: 3833 }], [{ v: 3456 }], [{ h: 3456 }], [], {}], [1, "hashicorp", 2, "^https://[a-z]*\\.hashicorp\\.com/", 22, [3834], [{ e: 3834 }], [{ v: 3834 }], [{ c: 3835 }, { c: 3836 }], [], {}], [1, "head-fi", 2, "^https?://(www\\.)?head-fi\\.org/", 22, [3837], [{ e: 3838 }], [{ v: 3837 }], [{ c: 3838 }], [{ timeout: 1e3, check: "none", wv: 3837 }], {}], [1, "hearst-us", 1, "^https://(www\\.)?(menshealth|womenshealthmag|cosmopolitan|esquire|elle|elledecor|harpersbazaar|goodhousekeeping|popularmechanics|caranddriver|delish|veranda|roadandtrack|bestproducts|countryliving|housebeautiful|bicycling|biography|pitchfork|prevention|runnersworld|thepioneerwoman|townandcountrymag|seventeen|oprahdaily|womansday|redbookmag|autoweek)\\.com/", 10, [3839], [{ e: 3839 }], [{ check: "any", v: 3839 }], [{ h: 3839 }], [{ check: "none", v: 3839 }], {}], [1, "heerlijkehuisjes", 2, "^https?://(\\w+\\.)?heerlijkehuisjes\\.nl/", 22, [3840], [{ e: 3841 }], [{ v: 3841 }], [{ c: 3842 }, { wv: 3843 }, { all: true, optional: true, k: 3844 }, { c: 3845 }], [{ cc: 3846 }], {}], [1, "hetzner.com", 2, "^https://www\\.hetzner\\.com/", 22, [3847], [{ e: 3847 }], [{ v: 3847 }], [{ k: 3848 }], [], {}], [1, "hikvision.com", 1, "^https?://(\\w+\\.)?hikvision\\.com/", 10, [], [{ e: 3849 }, { negated: true, e: 3850 }], [{ v: 3851 }], [{ h: 3851 }], [], {}], [1, "hsbc-au", 2, "^https://(www\\.)?hsbc\\.com\\.au/", 22, [3852], [{ e: 3853 }], [{ v: 3853 }], [{ c: 3854 }], [{ check: "none", timeout: 3e3, wv: 3853 }], {}], [1, "hyrabostad.se", 2, "^https?://(www\\.)?hyrabostad\\.se/", 22, [2660], [{ e: 3855 }], [{ v: 3855 }], [{ c: 3856 }, { wv: 3857 }, { if: { e: 3858 }, then: [{ k: 3858 }] }, { retry: 3, retryInterval: 400, c: 3859 }], [{ check: "none", v: 3860 }], {}], [1, "ibanez", 2, "^https?://www\\.ibanez\\.com/", 22, [3861], [{ e: 3862 }], [{ v: 3863 }], [{ c: 3862 }], [{ cc: 3864 }], {}], [1, "icom-co-jp", 1, "^https?://(www\\.)?icom\\.co\\.jp/", 22, [3865], [{ e: 3866 }], [{ v: 3867 }], [{ h: 1990 }], [{ check: "none", v: 1990 }], {}], [1, "idoctors.it", 2, "^https?://(www\\.)?idoctors\\.it/", 22, [3868, 3869], [{ e: 3868 }], [{ v: 3868 }], [{ c: 3870 }, { wv: 3869 }, { if: { e: 3871 }, then: [{ k: 3872 }] }, { retry: 3, retryInterval: 500, c: 3873 }], [{ cc: 3737 }], {}], [1, "ign.es", 1, "^https?://(www\\.)?ign\\.es/", 22, [3874], [{ e: 3875 }], [{ v: 3874 }], [{ h: 3874 }], [{ check: "none", v: 3874 }], {}], [1, "iisvaldagno", 1, "^https?://(www\\.)?iisvaldagno\\.it/", 22, [3876], [{ e: 3877 }], [{ v: 3876 }], [{ h: 3876 }], [{ check: "none", v: 3876 }], {}], [1, "imdb", 0, "^https://(www\\.)?(m\\.)?imdb.com/", 22, [3834], [{ e: 3834 }], [{ v: 3834 }], [{ c: 3878 }], [], {}], [1, "ing-de", 0, "^https://(\\w+\\.)?ing\\.de/", 10, [], [{ exists: ["ing-cc-manager", "ing-cc-dialog-level0"] }], [{ exists: ["ing-cc-manager[is-open]", "ing-cc-dialog-level0", ".cc-l0__button__more"] }], [{ waitForThenClick: ["ing-cc-manager", "ing-cc-dialog-level0", ".cc-l0__button__more"] }], [], {}], [1, "ing-nl", 2, "^https://([a-z0-9-]+\\.)?ing\\.nl/", 22, [], [{ exists: ["ing-app-open-page", "#cookieDialog", "ing-feat-cookie-dialog-nl"] }], [{ visible: ["ing-app-open-page", "#cookieDialog", "ing-feat-cookie-dialog-nl", "ing-feat-global-cookie-preference", "cookie-preference-main", "[data-testid=reject]"] }], [{ waitForThenClick: ["ing-app-open-page", "#cookieDialog", "ing-feat-cookie-dialog-nl", "ing-feat-global-cookie-preference", "cookie-preference-main", "[data-testid=reject]"] }], [{ cc: 3879 }], {}], [1, "instagram", 2, "^https://www\\.instagram\\.com/", 22, [], [{ e: 3880 }], [{ v: 3880 }], [{ c: 3881 }, { wait: 2e3 }], [], {}], [1, "intesasanpaolo", 2, "^https?://(\\w+\\.)?intesasanpaolo\\.com/", 22, [3882], [{ e: 3883 }], [{ v: 3883 }], [{ c: 3883 }], [{ cc: 3884 }, { negated: true, cc: 3885 }], {}], [1, "io.no", 0, "^https?://([a-z0-9-]+\\.)?io\\.no/", 22, [3886], [{ e: 3887 }], [{ v: 3886 }], [{ c: 3887 }], [{ check: "none", timeout: 1e3, wv: 3886 }], {}], [1, "ipkobiznes.pl", 2, "^https?://(www\\.)?ipkobiznes\\.pl/", 22, [3888], [{ e: 3888 }], [{ v: 3888 }], [{ c: 3889 }], [{ cc: 3890 }], {}], [1, "isidewith.com", 2, "^https?://(\\w+\\.)?isidewith\\.com/", 22, [], [{ e: 3891 }], [{ v: 3891 }], [{ c: 3892 }, { check: "none", timeout: 2e3, wv: 3891 }], [{ check: "none", v: 3891 }], {}], [2, "istore.co.za", 1, "^https?://(www\\.)?istore(preowned)?\\.co\\.za/", 22, [377], [{ e: 377 }], [{ v: 377 }], [{ h: 377 }, { optional: true, h: 3893 }, { removeClass: "no-scroll", selector: "body", optional: true }], [], {}], [1, "itopvpn.com", 1, "^https://(www\\.)?itopvpn.com/", 22, [], [{ e: 3894 }], [{ e: 3894 }], [{ h: 3894 }], [], {}], [1, "iubenda-rti", 2, "^https?://([\\w-]+\\.)*(mediaset\\.it|meteo\\.it|wittytv\\.it|today\\.it|milanotoday\\.it|romatoday\\.it|padovaoggi\\.it|bolognatoday\\.it|torinotoday\\.it|genovatoday\\.it|trevisotoday\\.it)/", 10, [1421], [{ e: 3895 }], [{ v: 3895 }], [{ c: 3895 }, { c: 3896 }], [], {}], [1, "iworks-footer-notice", 2, "^https?://(www\\.)?prorankingi\\.pl/", 22, [3897], [{ e: 3897 }], [{ v: 3897 }, { negated: true, cc: 3898 }], [{ timeout: 15e3, optional: true, w: 3899 }, { retry: 3, retryInterval: 1e3, c: 3900 }], [{ cc: 3898 }], {}], [1, "jetpens", 2, "^https?://(www\\.)?jetpens\\.com/", 22, [3901], [{ e: 3902 }], [{ v: 3901 }], [{ c: 3903 }, { wv: 3904 }, { all: true, optional: true, k: 3905 }, { k: 3906 }, { check: "none", timeout: 2e3, wv: 3901 }], [], {}], [1, "jobtoday", 2, "^https?://(www\\.)?jobtoday\\.com/", 22, [], [{ e: 3907 }], [{ v: 3907 }], [{ c: 3908 }, { wv: 3909 }, { all: true, optional: true, k: 3910 }, { c: 3911 }], [{ cc: 3912 }], {}], [1, "jurahilfe-de", 2, "^https?://(\\w+\\.)?jurahilfe\\.de/", 22, [3913], [{ e: 3914 }], [{ v: 3914 }], [{ all: true, optional: true, k: 3915 }, { c: 3916 }], [{ cc: 3917 }, { cc: 3918 }, { cc: 3919 }], {}], [1, "justgiving.com", 2, "^https://(?:[a-z0-9-]+\\.)?justgiving\\.com/", 22, [3920], [{ any: [{ e: 3920 }, { e: 3921 }] }], [{ v: 3920 }, { e: 3922 }], [{ c: 3922 }], [{ wait: 500 }, { cc: 3923 }], {}], [1, "justwatch.com", 2, "^https?://(\\w+\\.)?justwatch\\.com/", 22, [3924], [{ e: 3925 }], [{ v: 3925 }], [{ k: 3926 }, { c: 3927 }, { all: true, c: 3928 }, { all: true, optional: true, k: 3929 }, { wv: 3930 }, { k: 3930 }], [], {}], [1, "juzaphoto", 2, "^https?://(\\w+\\.)?juzaphoto\\.com/", 22, [3931], [{ e: 3931 }], [{ v: 3931 }], [{ c: 3932 }], [{ cc: 3933 }], {}], [1, "kaggle", 2, "^https?://(www\\.)?kaggle\\.com/", 22, [], [{ e: 3934 }], [{ v: 3934 }], [{ c: 3935 }], [{ cc: 3936 }], {}], [1, "kanopy", 2, "^https?://(www\\.)?kanopy\\.com/", 22, [], [{ e: 3937 }], [{ v: 3937 }], [{ if: { e: 3938 }, then: [{ c: 3938 }], else: [{ c: 3939 }] }, { wv: 3940 }, { if: { e: 3941 }, then: [{ k: 3941 }] }, { if: { e: 3942 }, then: [{ k: 3942 }] }, { if: { e: 3943 }, then: [{ k: 3943 }] }, { k: 3940 }], [], {}], [1, "kartverket", 0, "^https?://([a-z0-9-]+\\.)?kartverket\\.no/", 22, [3944], [{ e: 3945 }], [{ v: 3945 }], [{ c: 3945 }, { wv: 3946 }, { optional: true, k: 3947 }, { k: 3946 }], [{ check: "none", timeout: 1e3, wv: 3946 }], {}], [1, "kickstarter.com", 2, "^https?://(www\\.)?kickstarter\\.com/", 22, [3948], [{ e: 3949 }], [{ v: 3949 }], [{ c: 3950 }], [{ check: "none", v: 3949 }], {}], [1, "kingrecords-co-jp", 2, "^https?://(?:[^/]+\\.)?kingrecords\\.co\\.jp/", 22, [3951], [{ e: 3952 }], [{ v: 3952 }], [{ timeout: 1e4, optional: true, w: 3953 }, { c: 3952 }, { if: { v: 3954 }, then: [{ wait: 1e3 }, { k: 3952 }] }, { check: "none", timeout: 5e3, wv: 3954 }], [{ cc: 3955 }], {}], [1, "kiwico", 2, "^https?://(www\\.)?kiwico\\.com/", 10, [3956], [{ e: 3957 }], [{ v: 3958 }], [{ c: 3957 }, { c: 3959 }, { check: "none", timeout: 3e3, wv: 3960 }, { c: 3961 }], [{ check: "none", timeout: 3e3, wv: 3956 }], {}], [1, "kleinanzeigen-de", 2, "^https?://(www\\.)?kleinanzeigen\\.de", 22, [3962], [{ e: 3963 }], [{ v: 3963 }], [{ k: 3963 }], [], {}], [1, "klimatabelle.com", 2, "^https?://(www\\.)?klimatabelle\\.com/", 22, [929], [{ e: 3964 }], [{ v: 3964 }], [{ c: 3964 }], [{ cc: 3965 }], {}], [1, "kuendigung.org", 1, "^https?://(www\\.)?kuendigung\\.org/", 22, [3966], [{ e: 3967 }], [{ v: 3966 }], [{ h: 3966 }], [{ check: "none", v: 3966 }], {}], [1, "la-becanerie", 2, "^https://(www\\.)?la-becanerie\\.com/", 22, [3968], [{ e: 3969 }], [{ v: 3970 }], [{ c: 3971 }], [{ cc: 3972 }], {}], [1, "lacoccinelle", 2, "^https?://(\\w+\\.)?lacoccinelle\\.net/", 22, [1921], [{ e: 1921 }], [{ check: "any", v: 3973 }], [{ if: { e: 1923 }, then: [{ all: true, retry: 3, c: 1927 }, { c: 3974 }, { timeout: 5e3, optional: true, c: 3975 }], else: [{ c: 3976 }, { c: 3977 }, { check: "none", timeout: 1500, optional: true, wv: 1937 }, { if: { check: "any", v: 3978 }, then: [{ all: true, k: 3978 }, { check: "none", timeout: 5e3, wv: 1937 }] }] }], [{ check: "none", v: 3973 }], {}], [1, "lantmateriet", 2, "^https?://(www\\.)?lantmateriet\\.se/", 22, [3979], [{ e: 3980 }], [{ v: 3981 }], [{ c: 3980 }], [{ cc: 3982 }], {}], [1, "lastdodo", 2, "^https?://(www\\.)?lastdodo\\.(nl|com|de|fr)/", 22, [], [{ e: 3983 }], [{ v: 3984 }], [{ c: 3985 }, { wv: 3986 }, { all: true, optional: true, k: 3987 }, { c: 3988 }], [], {}], [3, "latrobe.edu.au", 1, "^https?://(\\w+\\.)?latrobe\\.edu\\.au/", 10, [3989], [{ e: 3990 }], [{ v: 3989 }], [{ h: 3989 }, { stylesheet: "body.gdpr-on #header { top: 0 !important; } body.vi #header #gdpr ~ #global-search-button, body.vi #header #gdpr ~ #global-search-button + label { top: 1.25rem; bottom: auto; }", stylesheetId: "latrobe-gdpr-layout" }], [], {}], [1, "le-livre.fr", 1, "^https?://(www\\.)?le-livre\\.fr/", 22, [3991], [{ e: 3992 }], [{ v: 3991 }], [{ h: 3991 }], [{ check: "none", v: 3991 }], {}], [1, "leafly", 1, "^https://(www\\.)?leafly\\.com/", 22, [], [{ e: 3993 }], [{ v: 3993 }], [{ h: 3993 }], [], {}], [1, "leetchi", 2, "^https?://(www\\.)?leetchi\\.com/", 22, [], [{ e: 3994 }], [{ v: 3995 }], [{ c: 3995 }, { timeout: 3e3, check: "none", wv: 3995 }], [{ cc: 3996 }], {}], [1, "legislation-gov-uk", 2, "^https?://(\\w+\\.)?legislation\\.gov\\.uk/", 22, [3997], [{ e: 3998 }], [{ v: 3998 }], [{ c: 3998 }, { timeout: 3e3, optional: true, c: 3999 }], [{ cc: 4e3 }], {}], [1, "lego", 2, "^https?://(www\\.)?lego\\.com/", 22, [4001], [{ any: [{ e: 4002 }, { e: 4003 }] }], [{ any: [{ v: 4002 }, { v: 4003 }] }], [{ any: [{ cc: 4004 }, { timeout: 3e3, wv: 4003 }], optional: true }, { check: "none", timeout: 12e4, wv: 4003 }, { c: 4002 }, { any: [{ cc: 4004 }, { timeout: 3e3, wv: 4003 }], optional: true }, { if: { v: 4003 }, then: [{ check: "none", timeout: 12e4, wv: 4003 }, { timeout: 5e3, c: 4002 }], optional: true }], [{ cc: 4005 }], {}], [1, "lendable", 2, "^https?://([\\w-]+\\.)?lendable\\.co\\.uk/", 22, [4006], [{ e: 4007 }], [{ v: 4006 }], [{ c: 4008 }, { c: 4009 }, { check: "none", wv: 4006 }], [{ cc: 594 }], {}], [1, "leukhuis-nl", 1, "^https?://(www\\.)?leukhuis\\.nl/", 22, [2582], [{ e: 4010 }], [{ v: 2582 }], [{ h: 2582 }], [{ check: "none", v: 2582 }], {}], [1, "lexbase.se", 1, "^https?://(www\\.)?lexbase\\.se/", 22, [4011], [{ e: 4012 }], [{ v: 4011 }], [{ h: 4011 }], [{ check: "none", v: 4011 }], {}], [1, "lezenvoordelijst.nl", 0, "^https?://(www\\.)?lezenvoordelijst\\.nl/", 10, [4013, 4014], [{ e: 4015 }], [{ v: 4016 }], [{ c: 4017 }], [{ cc: 4018 }], {}], [1, "libris.nl", 2, "^https?://(www\\.)?libris\\.nl/", 22, [4019], [{ e: 4020 }], [{ v: 4020 }], [{ if: { e: 4021 }, then: [{ all: true, k: 4021 }] }, { c: 4020 }], [{ cc: 4022 }], {}], [1, "librus", 2, "^https?://([a-z0-9-]+\\.)?librus\\.pl/", 22, [4023], [{ e: 4024 }], [{ v: 4024 }], [{ c: 4024 }, { w: 4025 }, { if: { e: 4026 }, then: [{ all: true, k: 4026 }] }, { c: 4027 }], [{ check: "none", timeout: 2e3, wv: 4028 }], {}], [1, "lieder.net", 2, "^https?://(www\\.)?lieder\\.net/", 22, [4029], [{ e: 4030 }], [{ v: 4031 }], [{ c: 4030 }], [{ cc: 4032 }], {}], [1, "livsmedelsverket", 2, "^https?://(\\w+\\.)?livsmedelsverket\\.se/", 22, [47], [{ e: 4033 }], [{ v: 4034 }], [{ c: 4033 }, { wv: 4035 }, { all: true, optional: true, k: 4036 }, { k: 4037 }], [{ cc: 4038 }], {}], [1, "loescher", 2, "^https?://([a-z0-9-]+\\.)?loescher\\.it/", 22, [], [{ e: 4039 }], [{ v: 4039 }], [{ c: 4040 }, { wv: 4041 }, { all: true, optional: true, k: 4042 }, { k: 4043 }], [{ cc: 4044 }], {}], [1, "lucozade.com", 2, "^https://(www\\.)?lucozade\\.com/", 22, [3603, 4045], [{ e: 3603 }], [{ v: 3603 }], [{ c: 4046 }], [{ cc: 4047 }], {}], [1, "lucrin", 2, "^https?://(www\\.)?lucrin\\.(com|com\\.au|co\\.uk|ca|ch|de|es|fr|it)/", 22, [4048, 4049], [{ e: 4050 }], [{ v: 377 }], [{ c: 4050 }, { wv: 4051 }, { all: true, optional: true, k: 4052 }, { c: 4051 }], [{ cc: 4053 }], {}], [1, "luxmed", 2, "^https?://([a-z0-9-]+\\.)?luxmed\\.pl/", 22, [], [{ e: 4054 }], [{ v: 4054 }], [{ c: 4055 }], [{ cc: 4056 }], {}], [1, "mackeeper", 0, "^https?://(\\w+\\.)?mackeeper\\.com/", 22, [4057], [{ e: 4058 }], [{ v: 4058 }], [{ c: 4059 }], [{ cc: 4060 }], {}], [1, "mainichi", 2, "^https?://(\\w+\\.)?mainichi\\.jp/", 22, [], [{ e: 4061 }, { negated: true, e: 1169 }], [{ v: 4061 }], [{ c: 4062 }], [{ check: "none", timeout: 2e3, wv: 4063 }], {}], [1, "mainlinemenswear", 2, "^https://(www\\.)?mainlinemenswear\\.co\\.uk/", 22, [4064], [{ e: 4065 }], [{ v: 4065 }], [{ c: 4066 }, { wv: 4067 }, { all: true, optional: true, k: 4068 }, { c: 4069 }], [], {}], [1, "makitatools", 2, "^https://(www\\.)?makitatools\\.com/", 22, [4070], [{ e: 4070 }], [{ v: 4070 }], [{ c: 4071 }, { c: 4072 }, { check: "none", wv: 4070 }], [{ check: "none", v: 4070 }], {}], [1, "malaysiaairlines.com", 0, "^https?://(\\w+\\.)?malaysiaairlines\\.com/", 22, [], [{ e: 4073 }], [{ v: 4073 }], [{ c: 4074 }, { wv: 4075 }, { all: true, optional: true, k: 4076 }, { c: 4077 }], [], {}], [1, "mango", 2, "^https://(shop\\.mango|www\\.mangooutlet)\\.com/", 22, [4078], [{ e: 4079 }], [{ v: 4079 }], [{ if: { e: 4080 }, then: [{ retry: 5, retryInterval: 500, c: 4080 }], else: [{ retry: 5, retryInterval: 500, c: 4081 }, { retry: 5, retryInterval: 500, c: 4082 }] }], [{ cc: 4083 }], {}], [1, "maree-info", 2, "^https?://(www\\.)?maree\\.info/", 22, [4084], [{ e: 4085 }], [{ v: 4085 }], [{ c: 4086 }, { check: "none", timeout: 3e3, optional: true, wv: 4085 }], [{ cc: 4087 }], {}], [1, "massaggiamilano", 2, "^https?://(www\\.)?massagg(iamilano|iemozionali)\\.it/", 22, [4088], [{ e: 4089 }], [{ v: 4089 }], [{ c: 4089 }, { c: 4090 }], [{ check: "none", timeout: 5e3, wv: 4091 }], {}], [1, "massgeneralbrigham", 0, "^https?://(\\w+\\.)?massgeneralbrigham\\.org/", 22, [4092], [{ e: 4093 }], [{ v: 4093 }], [{ c: 4093 }], [{ negated: true, e: 4092 }], {}], [1, "matemaks", 1, "^https?://(www\\.)?matemaks\\.pl/", 22, [4094], [{ e: 4095 }], [{ v: 4094 }], [{ h: 4094 }], [], {}], [1, "maticrobots-com", 2, "^https?://(\\w+\\.)?maticrobots\\.com/", 22, [4096], [{ e: 4096 }], [{ v: 4096 }], [{ c: 4097 }, { wv: 4098 }, { c: 4099 }, { c: 4100 }], [], {}], [1, "mblock", 1, "^https?://ide\\.mblock\\.cc/", 22, [4101], [{ e: 4102 }], [{ v: 4101 }], [{ h: 4101 }], [], {}], [1, "medium", 1, "^https://([a-z0-9-]+\\.)?medium\\.com/", 10, [], [{ e: 4103 }], [{ v: 4103 }], [{ h: 4103 }], [], {}], [1, "meduza", 0, "^https?://(\\w+\\.)?meduza\\.io/", 10, [4104], [{ e: 4105 }], [{ v: 4105 }], [{ c: 4105 }], [{ check: "none", timeout: 2e3, wv: 4104 }], {}], [1, "mega.io", 2, "^https?://(\\w+\\.)?mega\\.(io|nz)/", 22, [4106], [{ e: 4107 }], [{ v: 4106 }], [{ if: { v: 4108 }, then: [{ k: 4108 }], else: [{ k: 4107 }, { wv: 4109 }, { all: true, optional: true, k: 4110 }, { k: 4109 }] }], [{ cc: 3737 }], {}], [1, "menti.com", 2, "^https?://(www\\.)?menti\\.com/", 22, [4111], [{ e: 4112 }], [{ v: 4112 }], [{ retry: 10, retryInterval: 500, c: 4113 }], [{ cc: 4114 }], {}], [1, "meowdb.com", 2, "^https?://(\\w+\\.)?meowdb\\.com/", 22, [4115, 601], [{ e: 4116 }], [{ v: 4115 }], [{ c: 4116 }, { timeout: 3e3, optional: true, w: 602 }, { if: { e: 602 }, then: [{ wait: 500 }, { eval: "EVAL_CONSENTMANAGER_NCMP_REJECT" }] }], [{ cc: 4117 }], {}], [1, "merito.pl", 2, "^https?://(\\w+\\.)?merito\\.pl/", 22, [], [{ e: 4118 }], [{ v: 4119 }], [{ c: 4120 }, { wv: 4121 }, { all: true, optional: true, k: 4122 }, { k: 4123 }], [{ cc: 4124 }], {}], [1, "meta-cookie-dialog-v2", 2, "^https?://(\\w+\\.)?meta\\.com/", 22, [], [{ e: 4125 }], [{ v: 4125 }], [{ retry: 20, retryInterval: 500, c: 4125 }, { check: "none", wv: 4125 }], [], {}], [1, "midway-usa", 1, "^https://www\\.midwayusa\\.com/", 22, [4126], [{ e: 4126 }], [{ v: 4126 }], [{ h: 4126 }], [], {}], [1, "miles-and-more", 2, "^https://(www\\.)?miles-and-more\\.com/", 22, [4127], [{ e: 4128 }], [{ v: 4129 }], [{ wait: 500 }, { c: 4129 }], [{ cc: 4130 }], {}], [1, "mipro.com.tw", 2, "^https?://(\\w+\\.)?mipro\\.com\\.tw/", 22, [4131], [{ e: 4132 }], [{ v: 4133 }], [{ c: 4133 }], [{ negated: true, e: 4131 }], {}], [1, "mites.gob.es", 2, "^https?://(\\w+\\.)?mites\\.gob\\.es/", 22, [4134, 3555], [{ e: 4135 }], [{ v: 4135 }], [{ c: 4135 }, { wv: 4136 }, { all: true, optional: true, k: 4137 }, { k: 4136 }], [{ cc: 4138 }, { negated: true, cc: 4139 }], {}], [1, "mizuno.com", 2, "^https?://(\\w+\\.)?mizuno\\.com/", 22, [111], [{ e: 4140 }], [{ v: 111 }], [{ c: 4140 }], [{ cc: 4141 }], {}], [1, "mobile.de", 2, "^https?://([a-z0-9-]+\\.)?mobile\\.de/", 22, [4142], [{ e: 4142 }], [{ v: 4143 }], [{ retry: 3, retryInterval: 500, c: 4144 }], [{ wait: 500 }, { cc: 4145 }], {}], [1, "modelsociety", 2, "^https?://(www\\.)?modelsociety\\.com/", 22, [4146], [{ e: 4146 }], [{ v: 4146 }], [{ c: 4147 }, { timeout: 3e3, optional: true, c: 4148 }], [{ check: "none", timeout: 2e3, wv: 4146 }], {}], [1, "monex-co-jp", 2, "^https?://(\\w+\\.)?monex\\.co\\.jp/", 22, [4149], [{ e: 4150 }], [{ v: 4149 }], [{ c: 4150 }], [{ check: "none", v: 4149 }], {}], [1, "money.co.uk", 2, "^https?://(www\\.)?money\\.co\\.uk/", 10, [4151], [{ e: 4152 }], [{ v: 4152 }], [{ all: true, optional: true, k: 4153 }, { c: 4152 }], [{ cc: 3737 }], {}], [1, "moneybird", 2, "^https?://(\\w+\\.)?moneybird\\.(nl|com|de|be)/", 22, [], [{ exists: ["mb-cookie-modal-component", "#mb-cookie-modal"] }], [{ visible: ["mb-cookie-modal-component", "#mb-cookie-modal[open]"] }], [{ waitForThenClick: ["mb-cookie-modal-component", "#show-preferences"] }, { waitForVisible: ["mb-cookie-modal-component", "#preferences-view"] }, { if: { exists: ["mb-cookie-modal-component", "#check-analytical:checked"] }, then: [{ click: ["mb-cookie-modal-component", "#check-analytical"] }] }, { if: { exists: ["mb-cookie-modal-component", "#check-thirdparty:checked"] }, then: [{ click: ["mb-cookie-modal-component", "#check-thirdparty"] }] }, { waitForThenClick: ["mb-cookie-modal-component", "#save-preferences"] }], [{ cc: 4154 }, { cc: 4155 }], {}], [1, "mos-cudzoziemcy-gov-pl", 1, "^https?://mos\\.cudzoziemcy\\.gov\\.pl/", 22, [4156], [{ e: 4157 }], [{ v: 4157 }], [{ h: 4156 }], [], {}], [1, "mq.edu.au", 2, "^https?://(\\w+\\.)?mq\\.edu\\.au/", 10, [4158], [{ exists: ["mq-alert-cookies", ".alert"] }], [{ visible: ["mq-alert-cookies", ".alert"], check: "any" }], [{ if: { exists: ["mq-alert-cookies[variant=tiktok]", ".alert .button.secondary"] }, then: [{ waitForThenClick: ["mq-alert-cookies[variant=tiktok]", ".alert .button.secondary"] }], else: [{ waitForThenClick: ["mq-alert-cookies[variant=cookies]", ".alert .button-container button"] }] }], [{ waitForVisible: ["mq-alert-cookies", ".alert"], check: "none", timeout: 2e3 }], {}], [1, "mrchadd", 2, "^https?://(www\\.)?mrchadd\\.nl/", 22, [], [{ e: 4159 }], [{ v: 4159 }], [{ c: 4160 }, { wv: 4161 }, { all: true, optional: true, k: 4162 }, { c: 4163 }], [{ check: "none", v: 4159 }], {}], [1, "msn", 0, "^https?://(www\\.)?msn\\.com/", 10, [4164, 4165], [{ e: 4164 }], [{ v: 4164 }], [{ c: 4166 }], [{ cc: 2414 }], {}], [1, "myjobquote", 2, "^https?://(www\\.)?myjobquote\\.co\\.uk/", 22, [4167], [{ e: 4168 }], [{ v: 4168 }], [{ c: 4169 }, { wv: 4170 }, { all: true, optional: true, k: 4171 }, { k: 4170 }], [], {}], [1, "nba.com", 1, "^https://(www\\.)?nba\\.com/", 22, [4172], [{ e: 4172 }], [{ v: 4172 }], [{ h: 4172 }], [], {}], [1, "ndl-go-jp", 2, "^https://([a-z0-9-]+\\.)?ndl\\.go\\.jp/", 22, [4173], [{ e: 4174 }], [{ v: 4174 }], [{ c: 4175 }], [{ timeout: 2e3, check: "none", wv: 4174 }], {}], [1, "nederlandseloterij", 2, "^https?://([a-z0-9-]+\\.)?nederlandseloterij\\.nl/", 22, [4176, 4177], [{ e: 4178 }], [{ v: 4178 }], [{ c: 4179 }], [{ timeout: 2e3, check: "none", wv: 4180 }], {}], [1, "nestpensions", 0, "^https?://(www\\.)?nestpensions\\.org\\.uk/", 22, [4181], [{ e: 4182 }], [{ v: 4181 }], [{ c: 4182 }], [{ cc: 4183 }], {}], [1, "netbeat.de", 2, "^https://(www\\.)?netbeat\\.de/", 22, [4184], [{ e: 4184 }], [{ v: 4184 }], [{ c: 4185 }], [], {}], [1, "newsbreak", 0, "^https?://(www\\.)?newsbreak\\.com/", 10, [], [{ e: 4186 }], [{ v: 4186 }], [{ c: 4186 }], [{ check: "none", timeout: 2e3, wv: 4186 }], {}], [1, "newyorker-de", 2, "^https?://(www\\.)?newyorker\\.de/", 22, [], [{ e: 4187 }], [{ visible: ["cookie-banner", ".cookie-banner .cookie-banner-controls"] }], [{ waitForThenClick: ["cookie-banner", ".cookie-banner-controls > button:nth-child(2)"] }], [{ waitForVisible: ["cookie-banner", ".cookie-banner"], timeout: 1e3, check: "none" }], {}], [1, "nhm.ac.uk", 1, "^https?://(\\w+\\.)?nhm\\.ac\\.uk/", 22, [4188, 4189], [{ e: 4188 }], [{ v: 4188 }], [{ h: 4190 }], [], {}], [1, "nhnieuws", 2, "^https://(www\\.)?nhnieuws\\.nl/", 22, [4191], [{ e: 4192 }], [{ check: "any", v: 4192 }], [{ c: 4193 }], [{ eval: "EVAL_NHNIEUWS_TEST" }], {}], [1, "nhs-jobs", 2, "^https?://(\\w+\\.)?jobs\\.nhs\\.uk/", 22, [47], [{ e: 4194 }], [{ v: 4194 }], [{ c: 4194 }], [], {}], [1, "nhsapp", 2, "^https?://(\\w+\\.)?nhsapp\\.service\\.nhs\\.uk/", 22, [4195], [{ e: 4196 }], [{ v: 4197 }], [{ c: 4196 }], [{ timeout: 1e3, check: "none", wv: 4197 }], {}], [1, "niedziela-pl", 1, "^https?://([\\w-]+\\.)?niedziela\\.pl/", 22, [4198], [{ e: 4199 }], [{ v: 4199 }], [{ h: 4198 }], [{ check: "none", v: 4199 }], {}], [1, "nike", 2, "^https://(www\\.)?nike\\.com/", 22, [], [{ e: 4200 }], [{ v: 4200 }], [{ all: true, c: 4201 }, { c: 4202 }], [], {}], [1, "nocturnhq", 2, "^https?://(www\\.)?nocturnhq\\.com/", 22, [], [{ e: 4203 }], [{ v: 4204 }], [{ c: 4204 }], [{ check: "none", timeout: 2e3, wv: 4203 }], {}], [1, "noord-holland.nl", 2, "^https?://(\\w+\\.)?noord-holland\\.nl/", 22, [892], [{ e: 4205 }], [{ v: 892 }], [{ if: { e: 4206 }, then: [{ all: true, optional: true, k: 4207 }, { c: 4208 }], else: [{ c: 4209 }] }], [], {}], [1, "nordea", 2, "^https?://([a-z0-9-]+\\.)?nordea\\.(se|fi|no|dk)/", 22, [], [{ e: 4210 }], [{ v: 781 }], [{ all: true, optional: true, k: 4211 }, { c: 4212 }], [{ cc: 4213 }], {}], [1, "nordnet", 2, "^https?://(www\\.)?nordnet\\.(dk|se|no|fi)/", 22, [], [{ e: 4214 }], [{ v: 4214 }], [{ c: 4215 }], [{ cc: 4216 }], {}], [1, "northwell.edu", 1, "^https?://(\\w+\\.)?northwell\\.edu/", 22, [4217], [{ exists: ["nwhlit-gdpr-banner", ".nwhlit-gdpr-banner"] }], [{ visible: ["nwhlit-gdpr-banner", ".nwhlit-gdpr-banner"] }], [{ h: 4217 }], [], {}], [1, "nos.nl", 1, "^https://nos\\.nl/", 22, [1735], [{ e: 1735 }], [{ visible: ["ccm-notification"] }], [{ h: 1735 }], [], {}], [1, "nperf", 2, "^https?://(\\w+\\.)?nperf\\.com/", 22, [], [{ e: 4218 }], [{ v: 4218 }], [{ c: 4218 }], [{ cc: 4219 }], {}], [1, "nts-org-uk", 2, "^https?://(\\w+\\.)?nts\\.org\\.uk/", 22, [4220], [{ e: 4221 }], [{ v: 4222 }], [{ c: 4223 }, { timeout: 2e3, wv: 4221 }, { if: { e: 4224 }, then: [{ all: true, k: 4224 }] }, { k: 4221 }], [{ cc: 4225 }], {}], [1, "nutritionix.com", 0, "^https://(www\\.)?nutritionix\\.com/", 22, [4226], [{ e: 4226 }], [{ v: 4226 }], [{ waitForThenClick: ["gdpr-banner", "xpath///button[contains(., 'Refuse')]"] }], [], {}], [1, "octoparse", 2, "^https?://(\\w+\\.)?octoparse\\.(com|jp|de|fr|es|ai)/", 22, [4227], [{ e: 4228 }], [{ v: 4228 }], [{ c: 4228 }, { check: "none", timeout: 2e3, wv: 4227 }], [], {}], [1, "octopusenergy-it", 2, "^https?://(www\\.)?octopusenergy\\.it/", 22, [], [{ e: 4229 }], [{ v: 4229 }], [{ c: 4229 }, { w: 4230 }, { if: { e: 4231 }, then: [{ k: 4230 }] }, { if: { e: 4232 }, then: [{ k: 4233 }] }, { c: 4234 }], [{ cc: 4235 }], {}], [1, "ofsted-reports-cosmetic", 1, "^https?://reports\\.ofsted\\.gov\\.uk/", 22, [4236], [{ e: 4237 }], [{ v: 4237 }], [{ h: 4236 }], [{ check: "none", v: 4237 }], {}], [1, "ok", 0, "^https://(\\w+\\.)?ok\\.ru/", 22, [4238], [{ e: 4239 }], [{ v: 4239 }], [{ retry: 6, retryInterval: 1e3, c: 4240 }, { wv: 4241 }, { wait: 500 }, { all: true, optional: true, k: 4242 }, { c: 4243 }], [], {}], [1, "okayama-kanko-jp", 2, "^https?://(www\\.)?okayama-kanko\\.jp/", 22, [2891], [{ e: 4244 }], [{ v: 2891 }], [{ c: 4244 }, { check: "none", wv: 2891 }], [], {}], [1, "oko-press", 2, "^https?://(www\\.)?oko\\.press/", 22, [4245], [{ e: 4246 }], [{ v: 4245 }], [{ c: 4247 }, { wv: 4248 }, { if: { e: 4249 }, then: [{ k: 4248 }] }, { c: 4250 }], [{ check: "none", timeout: 2e3, wv: 4245 }, { cc: 4251 }], {}], [1, "olvg", 2, "^https?://(www\\.)?olvg\\.nl/", 22, [4252], [{ e: 4253 }], [{ v: 4254 }], [{ c: 4253 }, { c: 4255 }, { check: "none", timeout: 2e3, optional: true, wv: 4252 }], [{ cc: 4256 }], {}], [1, "ometv.pl", 2, "^https?://(\\w+\\.)?ometv\\.pl/", 22, [], [{ e: 4257 }], [{ v: 4258 }], [{ c: 4257 }], [{ check: "none", v: 4258 }], {}], [1, "omroepflevoland", 2, "^https?://(www\\.)?omroepflevoland\\.nl/", 22, [], [{ e: 4259 }], [{ check: "any", v: 4259 }], [{ if: { v: 4260 }, then: [{ k: 4261 }, { k: 4262 }, { if: { v: 4263 }, then: [{ k: 4263 }] }] }, { if: { v: 4264 }, then: [{ k: 4265 }] }], [{ check: "none", timeout: 2e3, wv: 4259 }], {}], [1, "onlineradiobox", 1, "^https?://(\\w+\\.)?onlineradiobox\\.com/", 22, [4266], [{ e: 4267 }], [{ v: 4266 }], [{ h: 4266 }], [{ check: "none", v: 4266 }], {}], [1, "onlyFans.com", 2, "^https://onlyfans\\.com/", 22, [4268], [{ e: 4268 }], [{ e: 4268 }], [{ k: 4269 }, { if: { e: 4270 }, then: [{ all: true, k: 4271 }, { k: 4272 }] }], [], {}], [1, "opalcollection-com", 1, "^https://(www\\.)?opalcollection\\.com/", 22, [4273], [{ v: 4274 }], [{ v: 4273 }], [{ h: 4273 }], [], {}], [1, "openai", 0, "^https://([a-z0-9-]+\\.)?openai\\.com/", 22, [4275], [{ e: 4275 }], [{ v: 4275 }], [{ c: 4276 }], [{ wait: 500 }, { cc: 4277 }], {}], [1, "opera.com", 0, "^https?://(www\\.|)?opera\\.com/", 22, [550], [{ e: 4278 }], [{ v: 4278 }], [{ all: true, timeout: 500, optional: true, c: 4279 }, { c: 4280 }], [{ cc: 4281 }, { negated: true, cc: 4282 }], {}], [1, "optum", 2, "^https?://(\\w+\\.)?optum\\.com/", 22, [4283], [{ e: 4284 }], [{ v: 4284 }], [{ c: 4284 }], [{ check: "none", timeout: 2e3, wv: 4283 }], {}], [1, "optyczne-pl", 1, "^https?://(\\w+\\.)?optyczne\\.pl/", 22, [4285], [{ e: 4286 }], [{ v: 4285 }], [{ h: 4285 }], [], {}], [1, "orbys", 1, "^https?://(\\w+\\.)?orbys\\.eu/", 22, [4287], [{ e: 4288 }], [{ v: 4287 }], [{ h: 4287 }], [{ check: "none", v: 4287 }], {}], [1, "oresundstag", 2, "^https?://(www\\.)?oresundstag\\.se/", 22, [4289], [{ e: 4290 }], [{ v: 4291 }], [{ all: true, optional: true, k: 4292 }, { c: 4293 }], [{ cc: 2423 }], {}], [1, "osf.io", 1, "^https?://(\\w+\\.)?osf\\.io/", 22, [4294], [{ e: 4295 }], [{ v: 4295 }], [{ h: 4294 }], [], {}], [1, "oslosportslager.no", 0, "^https?://(www\\.)?oslosportslager\\.no/", 22, [4296], [{ e: 4297 }], [{ v: 4296 }], [{ c: 4297 }], [{ cc: 4298 }], {}], [1, "ourworldindata", 2, "^https://ourworldindata\\.org/", 22, [4299], [{ e: 4299 }], [{ v: 4300 }], [{ c: 4301 }], [], {}], [1, "pakamera.pl", 2, "^https?://(\\w+\\.)?pakamera\\.pl/", 22, [4302], [{ e: 4302 }], [{ v: 4303 }], [{ c: 4304 }, { wv: 4305 }, { all: true, optional: true, k: 4306 }, { k: 4305 }], [{ check: "none", wv: 4303 }], {}], [1, "palace-company", 1, "^https://(www\\.)?(moonpalace|palaceresorts|leblancsparesorts|baglionihotels)\\.com/", 22, [550, 4307], [{ e: 4308 }], [{ v: 4308 }], [{ h: 4309 }], [], {}], [1, "paychex", 1, "^https://(www\\.)?paychex\\.com/", 22, [4310], [{ e: 4310 }], [{ v: 4310 }], [{ h: 4310 }], [], {}], [1, "payfip", 1, "^https?://(www\\.)?payfip\\.gouv\\.fr/", 22, [4311], [{ e: 4312 }], [{ check: "any", v: 4311 }], [{ h: 4311 }], [{ check: "none", v: 4311 }], {}], [1, "pcloud", 2, "^https?://(\\w+\\.)?pcloud\\.com/", 22, [], [{ e: 4313 }], [{ v: 4313 }], [{ c: 4314 }, { wv: 4315 }, { all: true, optional: true, k: 4316 }, { k: 4317 }], [{ cc: 4318 }], {}], [1, "pcspecialist", 2, "^https?://(www\\.)?pcspecialist\\.[a-z.]+/", 22, [4319, 4320], [{ e: 4321 }], [{ v: 4320 }], [{ c: 4321 }], [{ cc: 4322 }], {}], [1, "pekao24", 2, "^https?://(www\\.)?pekao24\\.pl/", 22, [4323], [{ e: 4324 }], [{ v: 4324 }], [{ c: 4325 }], [{ cc: 4326 }], {}], [1, "pensionsmyndigheten", 2, "^https?://(\\w+\\.)?pensionsmyndigheten\\.se/", 22, [4327], [{ e: 4327 }], [{ v: 4327 }], [{ c: 4328 }], [{ cc: 4329 }], {}], [1, "pfron-gov-pl", 2, "^https?://(www\\.)?pfron\\.(gov|org)\\.pl/", 22, [3812], [{ e: 4330 }], [{ v: 4330 }], [{ retry: 3, retryInterval: 500, c: 4330 }], [{ cc: 4331 }], {}], [1, "pharmasave", 2, "^https?://(www\\.)?pharmasave\\.com/", 22, [1319], [{ e: 4332 }], [{ v: 4332 }], [{ retry: 10, retryInterval: 500, c: 4333 }, { check: "none", wv: 1319 }], [{ cc: 4334 }], {}], [1, "pinterest-business", 2, "^https://[a-z]*\\.pinterest\\.com/", 22, [4335], [{ e: 4335 }], [{ v: 4336 }], [{ c: 4337 }], [], {}], [1, "pitt-edu", 1, "^https?://(\\w+\\.)?pitt\\.edu/", 22, [4338], [{ e: 4338 }], [{ v: 4338 }], [{ h: 4338 }], [], {}], [1, "pizzapilgrims.co.uk", 2, "^https?://(www\\.)?pizzapilgrims\\.co\\.uk/", 10, [4339], [{ e: 4340 }], [{ v: 4341 }], [{ c: 4342 }, { wv: 4343 }, { all: true, optional: true, k: 4344 }, { k: 4345 }], [{ cc: 4346 }, { cc: 4347 }], {}], [1, "plex.tv", 2, "^https?://(\\w+\\.)?plex\\.tv/", 22, [4348, 4349, 4350], [{ any: [{ e: 4348 }, { e: 4351 }, { e: 4349 }, { e: 4350 }] }], [{ any: [{ v: 4348 }, { v: 4351 }, { v: 4349 }, { v: 4350 }] }], [{ if: { v: 4351 }, then: [{ c: 4352 }, { timeout: 5e3, optional: true, c: 4353 }], else: [{ if: { v: 4350 }, then: [{ c: 4354 }], else: [{ c: 4355 }] }] }], [{ cc: 4356 }], {}], [1, "plos", 0, "^https://([.a-zA-Z0-9-]+\\.)?plos\\.org/", 22, [550], [{ e: 4357 }], [{ v: 4357 }], [{ all: true, optional: true, k: 4358 }, { waitForThenClick: ["#cookie-consent", "xpath///button[contains(., 'Save Selected')]"] }], [{ cc: 4359 }], {}], [1, "podimo", 2, "^https?://(www\\.)?podimo\\.com/", 22, [], [{ e: 4360 }], [{ check: "any", v: 4360 }], [{ if: { v: 4361 }, then: [{ c: 4361 }], else: [{ c: 4362 }] }, { wv: 4363 }, { all: true, optional: true, k: 4364 }, { c: 4365 }], [{ cc: 4366 }, { negated: true, cc: 4367 }], {}], [1, "poki", 2, "^https?://(\\w+\\.)?poki\\.com/", 22, [4368], [{ e: 4369 }], [{ v: 4369 }], [{ c: 4369 }], [{ check: "none", timeout: 2e3, wv: 4370 }], {}], [1, "pornhub-compact-cookie-banner", 0, "^https://(\\w+\\.)?pornhub\\.(com|org)/", 22, [4371], [{ e: 4372 }], [{ v: 4372 }], [{ k: 4372 }], [], {}], [1, "pornhub.com", 0, "^https://(\\w+\\.)?pornhub\\.(com|org)/", 22, [4373], [{ e: 4374 }], [{ v: 4374 }], [{ if: { e: 4375 }, then: [{ c: 4375 }] }, { if: { e: 4376 }, then: [{ k: 4376 }], else: [{ c: 4377 }] }], [], {}], [1, "pornpics.de", 2, "^https?://(www\\.)?pornpics\\.(com|de)/", 22, [], [{ e: 4378 }], [{ v: 4378 }], [{ c: 4379 }], [{ cc: 4380 }], {}], [1, "posthog", 2, "^https?://(www\\.)?posthog\\.com/", 22, [], [{ e: 4381 }], [{ v: 4381 }], [{ c: 4382 }], [{ check: "none", timeout: 2e3, wv: 4381 }], {}], [2, "postnl", 0, "^https://([a-z]*\\.)?postnl\\.nl/", 22, [4383], [{ e: 4383 }], [{ visible: ["pnl-cookie-wall-widget", ".pnl-cookie-wall"] }], [{ waitForThenClick: ["pnl-cookie-wall-widget", ".cookie-overview__footer button.stamp-button--variant-secondary:last-child"] }, { waitForVisible: ["pnl-cookie-wall-widget", ".pnl-cookie-wall"], check: "none", timeout: 3e3, optional: true }, { setStyle: "", selector: "body", optional: true }], [{ cc: 4384 }], {}], [1, "povr", 0, "^https://povr\\.com/", 22, [], [{ e: 4385 }], [{ v: 4385 }], [{ h: 4386 }, { if: { e: 4387 }, then: [{ w: 4388 }, { all: true, optional: true, k: 4389 }, { c: 4390 }, { eval: "EVAL_POVR_GOBACK" }], else: [{ c: 4391 }] }], [], {}], [1, "praca-pl", 2, "^https?://(www\\.)?praca\\.pl/", 22, [4392], [{ e: 4393 }], [{ v: 4393 }], [{ c: 4393 }, { wv: 4394 }, { k: 4393 }], [{ check: "none", wv: 4395 }], {}], [1, "praktikertjanst", 2, "^https?://(www\\.)?praktikertjanst\\.se/", 22, [4396], [{ e: 4397 }], [{ v: 4398 }], [{ all: true, optional: true, k: 4399 }, { c: 4398 }], [{ cc: 4400 }], {}], [1, "productz.com", 2, "^https://productz\\.com/", 22, [], [{ e: 4401 }], [{ v: 4401 }], [{ c: 4402 }], [], {}], [1, "professoren-tum-de", 2, "^https?://(www\\.)?professoren\\.tum\\.de/", 22, [3304], [{ e: 4403 }], [{ v: 4403 }], [{ c: 4404 }], [{ cc: 1569 }], {}], [1, "profiler-dk", 1, "^https?://(www\\.)?(profiler|debtor)\\.dk/", 22, [4405], [{ e: 4406 }], [{ v: 4405 }], [{ h: 4405 }], [], {}], [1, "properstar", 2, "^https?://((www\\.)?properstar\\.[a-z.]+|international\\.immobiliare\\.it)/", 22, [4407], [{ e: 4408 }], [{ v: 4409 }], [{ timeout: 15e3, w: 4410 }, { c: 4408 }, { timeout: 3e3, wv: 4411 }, { all: true, optional: true, k: 4412 }, { retry: 2, c: 4413 }], [{ check: "none", timeout: 2e3, wv: 4409 }], {}], [1, "publicmobile", 2, "^https?://(www\\.)?publicmobile\\.ca/", 22, [4414], [{ e: 4414 }], [{ v: 4415 }], [{ c: 4416 }, { wv: 4417 }, { all: true, optional: true, k: 4418 }, { c: 4419 }, { check: "none", timeout: 3e3, wv: 4415 }], [{ cc: 4420 }], {}], [1, "puesc-gov-pl", 2, "^https?://(www\\.)?puesc\\.gov\\.pl/", 22, [4421], [{ e: 4422 }], [{ v: 4422 }], [{ retry: 3, retryInterval: 500, c: 4422 }], [{ cc: 4423 }], {}], [1, "qudosbank-com-au", 2, "^https?://(www\\.)?qudosbank\\.com\\.au/", 22, [4424], [{ e: 4425 }], [{ v: 4424 }], [{ c: 4425 }, { check: "none", timeout: 2e3, wv: 4424 }], [], {}], [3, "racv", 1, "^https?://(www\\.)?racv\\.com\\.au/", 10, [4426], [{ e: 4427 }], [{ v: 4427 }], [{ h: 4426 }, { stylesheet: "body.gdpr-active { overflow: auto !important; } body.gdpr-active::before { display: none !important; }", stylesheetId: "racv-gdpr-backdrop" }], [], {}], [1, "random-org", 2, "^https?://(\\w+\\.)?random\\.org/", 22, [47], [{ e: 4428 }], [{ v: 47 }], [{ if: { e: 4429 }, then: [{ k: 4430 }] }, { if: { e: 4431 }, then: [{ k: 4432 }] }, { c: 4433 }], [{ cc: 4434 }], {}], [1, "raspberrypi.com", 0, "^https://([a-z0-9-]+\\.)?raspberrypi\\.com/", 10, [], [{ e: 4435 }], [{ v: 4435 }], [{ c: 4435 }], [{ check: "none", v: 4435 }], {}], [1, "readly", 2, "^https://([a-z0-9-]+\\.)?readly\\.(com|co)/", 22, [4436, 4437], [{ e: 4438 }], [{ check: "any", v: 4438 }], [{ if: { e: 4439 }, then: [{ c: 4439 }], else: [{ c: 4440 }, { if: { e: 4441 }, then: [{ all: true, c: 4441 }], else: [] }, { c: 4442 }] }], [], {}], [1, "reddit.com", 2, "^https://(www|old)\\.reddit\\.com/", 22, [4443], [{ e: 4444 }], [{ v: 4444 }], [{ c: 4445 }], [{ cc: 4446 }], {}], [1, "refurbed", 2, "^https?://(www\\.)?refurbed\\.(com|co\\.uk|[a-z]{2})/", 22, [4447], [{ e: 4448 }], [{ v: 4447 }], [{ c: 4448 }, { check: "none", timeout: 2e3, wv: 4447 }], [{ cc: 4449 }, { cc: 4450 }], {}], [1, "regjeringen.no", 2, "^https?://(www\\.)?regjeringen\\.no/", 22, [4451], [{ e: 4452 }], [{ v: 4453 }], [{ c: 4454 }], [{ check: "none", timeout: 2e3, wv: 4451 }], {}], [1, "rejestr.io", 0, "^https?://(www\\.)?rejestr\\.io/", 22, [], [{ e: 4455 }], [{ v: 4455 }], [{ c: 4455 }], [{ timeout: 1e3, check: "none", wv: 4455 }], {}], [1, "remarkable.com", 1, "^https://(www\\.)?remarkable\\.com/", 22, [4456], [{ e: 4457 }], [{ v: 4457 }], [{ h: 4456 }], [], {}], [1, "renaud-bray", 2, "^https?://(www\\.)?renaud-bray\\.com/", 22, [47], [{ e: 4458 }], [{ v: 47 }], [{ c: 4459 }, { wv: 4460 }, { c: 4461 }], [{ check: "none", v: 47 }, { check: "none", v: 4460 }], {}], [1, "restaurants.nl", 2, "^https?://(\\w+\\.)?restaurants\\.nl/", 22, [], [{ e: 4462 }], [{ v: 4462 }], [{ c: 4463 }], [{ check: "none", v: 4462 }], {}], [1, "reuma.nl", 2, "^https?://(www\\.)?reuma\\.nl/", 22, [4464], [{ e: 4465 }], [{ v: 4465 }], [{ if: { e: 4466 }, then: [{ k: 4466 }] }, { w: 4467 }, { all: true, optional: true, k: 4468 }, { c: 4465 }], [{ cc: 4469 }], {}], [1, "reverb", 2, "^https?://(www\\.)?reverb\\.com/", 22, [4470], [{ e: 4471 }], [{ v: 4471 }], [{ if: { e: 4472 }, then: [{ c: 4472 }], else: [{ c: 4473 }, { w: 4474 }, { all: true, optional: true, k: 4475 }, { c: 4476 }] }], [{ cc: 4477 }], {}], [1, "reverb-gdpr-nag", 2, "^https?://(www\\.)?reverb\\.com/", 22, [], [{ cc: 4478 }, { e: 4479 }], [{ v: 4480 }], [{ retry: 5, retryInterval: 500, c: 4479 }], [], {}], [1, "reviewsolicitors.co.uk", 2, "^https?://(www\\.)?reviewsolicitors\\.co\\.uk/", 22, [4481, 4482], [{ e: 4483 }], [{ v: 4481 }], [{ c: 4484 }, { wv: 4485 }, { wait: 500 }, { k: 4485 }], [{ cc: 4486 }], {}], [1, "rinkabyror-se", 1, "^https?://(www\\.)?rinkabyror\\.se/", 22, [4487], [{ e: 4488 }], [{ v: 4487 }], [{ h: 4487 }], [], {}], [1, "rittal", 2, "^https?://(\\w+\\.)?rittal\\.com/", 22, [4045], [{ e: 4489 }], [{ v: 4489 }], [{ c: 4490 }], [{ cc: 4491 }], {}], [1, "roblox", 0, "^https://(www\\.)?roblox\\.com/", 10, [], [{ e: 4492 }], [{ v: 4493 }], [{ c: 4494 }], [{ cc: 4495 }], {}], [1, "roblox-creator-hub", 0, "^https://create\\.roblox\\.com/", 10, [], [{ e: 4496 }], [{ v: 4497 }], [{ c: 4497 }, { check: "none", timeout: 2e3, optional: true, wv: 4497 }, { timeout: 3e3, optional: true, c: 4497 }], [{ cc: 4498 }], {}], [1, "rog-forum.asus.com", 2, "^https://rog-forum\\.asus\\.com/", 22, [4499], [{ e: 4499 }], [{ v: 4499 }], [{ k: 4500 }, { c: 4501 }], [], {}], [1, "roll20", 2, "^https://([a-z0-9-]+\\.)?roll20\\.net/", 22, [4502], [{ e: 4503 }], [{ v: 4502 }], [{ c: 4504 }, { timeout: 2e3, wv: 4505 }, { if: { e: 4506 }, then: [{ k: 4507 }] }, { k: 4508 }], [{ cc: 4509 }], {}], [1, "romeo", 2, "^https?://(www\\.)?romeo\\.com/", 22, [], [{ e: 4510 }], [{ v: 4510 }], [{ c: 4511 }, { wv: 4512 }, { if: { e: 4513 }, then: [{ k: 4512 }] }, { k: 4514 }, { check: "none", wv: 4510 }], [], {}], [1, "roofingmegastore.co.uk", 2, "^https://(www\\.)?roofingmegastore\\.co\\.uk", 22, [4515], [{ e: 4515 }], [{ v: 4515 }], [{ k: 4516 }, { c: 4517 }], [], {}], [1, "route-inn", 2, "^https?://(\\w+\\.)?(route-inn|ark-hotel)\\.co\\.jp/", 22, [3416], [{ e: 4518 }], [{ v: 4519 }], [{ c: 4518 }], [{ cc: 4520 }], {}], [1, "rspb.org.uk", 0, "^https://(\\w+\\.)?rspb\\.org\\.uk/", 10, [4521], [{ e: 4521 }], [{ v: 4521 }], [{ c: 4522 }, { wv: 4523 }, { c: 4524 }], [{ cc: 4525 }], {}], [1, "rt", 1, "^https://(www\\.)?rt\\.com/", 22, [4526], [{ e: 4526 }], [{ v: 4526 }], [{ h: 4526 }], [], {}], [1, "rug-nl", 0, "^https?://(\\w+\\.)?rug\\.nl/", 10, [4527], [{ e: 4528 }], [{ v: 4527 }], [{ c: 4528 }], [], {}], [1, "rule34-xxx", 2, "^https?://(www\\.)?rule34\\.xxx/", 22, [4529], [{ e: 4529 }], [{ v: 4529 }], [{ c: 4530 }], [{ cc: 4531 }], {}], [1, "rutjes", 2, "^https?://(www\\.)?rutjes(paardenboxen\\.nl|pferdeboxen\\.de)/", 22, [4447], [{ e: 4532 }], [{ v: 4533 }], [{ c: 4533 }], [{ wait: 500 }, { cc: 4534 }], {}], [1, "rwth-moodle", 2, "^https?://moodle\\.rwth-aachen\\.de/", 22, [1217], [{ e: 4535 }], [{ v: 1217 }], [{ c: 4535 }], [{ cc: 4536 }], {}], [1, "ryanair", 0, "^https://(www\\.)?ryanair\\.com/", 10, [4537], [{ e: 4537 }], [{ v: 4537 }], [{ c: 4538 }], [{ cc: 4539 }], {}], [1, "samsung.com", 1, "^https://www\\.samsung\\.com/", 22, [4540], [{ e: 4540 }], [{ v: 4540 }], [{ h: 4540 }], [], {}], [1, "san-tatsu.jp", 2, "^https?://(\\w+\\.)?san-tatsu\\.jp/", 22, [4541], [{ e: 4542 }], [{ v: 4542 }], [{ c: 4542 }], [{ cc: 4543 }], {}], [1, "saos-org-pl", 1, "^https?://(www\\.)?saos\\.org\\.pl/", 22, [4544, 4545], [{ e: 4546 }], [{ v: 4544 }], [{ h: 4547 }], [{ check: "none", v: 4544 }], {}], [1, "saturdaysnyc", 2, "^https?://(www\\.)?saturdaysnyc\\.com/", 22, [4548], [{ e: 4549 }], [{ v: 4548 }], [{ c: 4549 }], [{ cc: 4550 }], {}], [1, "scb.se", 2, "^https?://(www\\.)?scb\\.se/", 22, [4551], [{ e: 4552 }], [{ v: 4551 }], [{ waitForThenClick: ["#kakrutan scb-button[value='inget']", "button"] }], [{ check: "none", v: 4551 }], {}], [1, "schadefonds", 2, "^https?://(www\\.)?schadefonds\\.nl/", 22, [3304], [{ e: 4553 }], [{ v: 4553 }], [{ timeout: 2e3, optional: true, w: 4554 }, { c: 3306 }], [], {}], [1, "schoolhouse-com", 1, "^https://(www\\.)?schoolhouse\\.com/", 22, [4555], [{ e: 4555 }], [{ v: 4555 }], [{ h: 4555 }], [], {}], [1, "schumacher.com", 2, "^https?://(www\\.)?schumacher\\.com/", 22, [], [{ e: 4556 }], [{ v: 4556 }], [{ if: { e: 4557 }, then: [{ k: 4557 }], else: [{ c: 4558 }, { c: 4559 }] }], [{ cc: 4560 }, { cc: 4561 }, { cc: 4562 }], {}], [1, "schuurman-schoenen", 2, "^https?://(www\\.)?schuurman-(schoenen|schuhe)\\.(nl|be|de)/", 10, [4563], [{ e: 4564 }], [{ v: 4565 }], [{ c: 4566 }, { c: 4564 }], [{ cc: 4567 }], {}], [1, "scmp", 2, "^https://(www\\.)?scmp\\.com/", 22, [4568], [{ any: [{ e: 4569 }, { e: 1169 }] }], [{ any: [{ v: 4569 }, { v: 2802 }] }], [{ timeout: 4e3, optional: true, wv: 2802 }, { if: { v: 2802 }, then: [{ k: 1171 }, { all: true, optional: true, k: 1173 }, { optional: true, k: 1174 }] }, { timeout: 3e3, optional: true, wv: 4569 }, { h: 4568 }], [], {}], [1, "scotland-police-uk", 1, "^https?://(www\\.)?scotland\\.police\\.uk/", 22, [4570], [{ e: 4571 }], [{ v: 4571 }], [{ h: 4570 }], [{ check: "none", v: 4570 }], {}], [1, "sda-it", 2, "^https?://(\\w+\\.)?sda\\.it/", 22, [4572], [{ e: 4573 }], [{ v: 4573 }], [{ c: 4573 }], [{ cc: 4574 }, { negated: true, cc: 4575 }], {}], [1, "seg-social", 2, "^https?://([a-z0-9-]+\\.)*seg-social\\.(gob\\.)?es/", 22, [4576], [{ e: 4577 }], [{ v: 4577 }], [{ c: 4577 }], [{ cc: 4578 }], {}], [1, "seniorweb", 2, "^https?://(www\\.)?seniorweb\\.nl/", 22, [4579], [{ e: 4579 }], [{ v: 4579 }], [{ c: 4580 }, { wv: 4581 }, { k: 4581 }, { c: 4582 }], [{ cc: 4583 }], {}], [1, "seria-group", 1, "^https?://(www\\.)?seria-group\\.com/", 22, [4584], [{ e: 4585 }], [{ v: 4586 }], [{ h: 4584 }], [{ check: "none", v: 4584 }], {}], [1, "serpstat-com", 1, "^https://(\\w+\\.)?serpstat\\.com/", 22, [4587], [{ e: 4588 }], [{ v: 4588 }], [{ h: 4587 }], [], {}], [1, "service2school", 2, "^https?://(www\\.)?service2school\\.org/", 22, [2163, 1344], [{ e: 2171 }, { negated: true, cc: 4589 }], [{ v: 2165 }], [{ h: 2163 }, { timeout: 5e3, c: 1345 }], [{ cc: 4590 }], {}], [1, "sex.com", 2, "^https?://(\\w+\\.)?sex\\.com/", 22, [4591], [{ e: 4592 }], [{ v: 4593 }], [{ w: 4594 }, { c: 4595 }, { c: 4596 }], [{ cc: 858 }], {}], [1, "sfanytime", 2, "^https?://(www\\.)?sfanytime\\.com/", 22, [3634], [{ e: 3634 }], [{ v: 3634 }], [{ c: 4597 }, { wv: 4598 }, { all: true, optional: true, k: 4599 }, { c: 4600 }], [{ check: "none", v: 3634 }], {}], [1, "shein.com", 2, "^https?://([a-z]+\\.)?shein\\.com/", 22, [4601], [{ e: 4602 }], [{ v: 4602 }], [{ c: 4602 }], [{ timeout: 1e3, check: "none", wv: 4602 }], {}], [1, "shimano-bike", 1, "^https?://bike\\.shimano\\.com/", 22, [4603], [{ e: 4604 }], [{ v: 4603 }], [{ h: 4603 }], [], {}], [1, "shinden.pl", 2, "^https?://(www\\.)?shinden\\.pl/", 22, [617], [{ e: 4605 }], [{ check: "any", v: 4605 }], [{ h: 617 }, { timeout: 1e4, optional: true, wv: 2371 }, { if: { v: 2371 }, then: [{ retry: 2, retryInterval: 500, c: 2372 }, { c: 2373 }, { check: "none", wv: 2371 }] }], [], {}], [1, "shoezone", 1, "^https?://(www\\.)?shoezone\\.com/", 22, [4606], [{ e: 4607 }], [{ v: 4606 }], [{ h: 4606 }], [{ check: "none", v: 4606 }], {}], [1, "shopquarters", 1, "^https?://(www\\.)?shopquarters\\.com/", 22, [2070], [{ e: 4608 }], [{ check: "any", v: 4609 }], [{ h: 4608 }, { timeout: 2e3, optional: true, c: 2071 }], [], {}], [1, "showroomprive", 2, "^https?://(\\w+\\.)*(showroomprive\\.\\w+|saldiprivati\\.com)/", 22, [4610, 4611], [{ e: 4612 }], [{ v: 4613 }], [{ c: 4612 }], [{ cc: 4614 }], {}], [1, "silver.nu", 2, "^https?://(www\\.)?silver\\.nu/", 22, [], [{ e: 4615 }], [{ v: 4615 }], [{ c: 4616 }, { timeout: 5e3, optional: true, w: 1169 }, { if: { e: 1169 }, then: [{ if: { e: 1171 }, then: [{ k: 1171 }] }, { all: true, optional: true, k: 1173 }, { optional: true, k: 1174 }] }], [{ cc: 4617 }], {}], [1, "simplehuman-com", 2, "^https?://(www\\.)?simplehuman\\.com/", 10, [4618], [{ e: 4619 }], [{ v: 4620 }], [{ c: 4619 }, { c: 4621 }], [], {}], [1, "simyo-nl", 2, "^https?://(www\\.)?simyo\\.nl/", 10, [4622], [{ v: 4622 }], [{ v: 4622 }], [{ c: 4623 }], [{ cc: 4624 }], {}], [1, "skatteverket-se", 2, "^https?://(www\\.)?skatteverket\\.se/", 22, [4625], [{ exists: ["skv-cookie-banner", ".skv-cookie-banner-root"] }], [{ visible: ["skv-cookie-banner", ".skv-cookie-banner-root"] }], [{ waitForThenClick: ["skv-cookie-banner", "#deny-all"] }], [{ cc: 4626 }], {}], [1, "skyscanner", 0, "^https://(www\\.)?skyscanner[\\.a-z]+/", 10, [4492], [{ e: 4627 }], [{ v: 4627 }], [{ c: 4628 }, { check: "none", wv: 4627 }], [{ eval: "EVAL_SKYSCANNER_TEST" }], {}], [1, "sl-se", 2, "^https?://(www\\.)?sl\\.se/", 22, [4629], [{ e: 4630 }], [{ v: 4630 }], [{ c: 4630 }], [{ cc: 4631 }], {}], [1, "smallpdf", 2, "^https?://(www\\.)?smallpdf\\.com/", 22, [], [{ e: 4632 }], [{ v: 4632 }], [{ c: 4633 }, { wv: 4634 }, { all: true, optional: true, k: 4635 }, { k: 4634 }], [{ cc: 4636 }], {}], [1, "smile-insurances", 2, "^https?://(www\\.)?smile-insurances\\.com/", 22, [4637], [{ e: 4637 }], [{ v: 4638 }], [{ c: 4638 }], [{ cc: 4639 }], {}], [1, "snb.ch", 2, "^https?://(www\\.)?snb\\.ch/", 22, [4640, 4641], [{ e: 4642 }], [{ v: 4643 }], [{ c: 4642 }, { wv: 4644 }, { all: true, optional: true, k: 4645 }, { k: 4644 }], [{ check: "none", v: 4640 }], {}], [1, "solvhealth.com", 2, "^https://(www\\.)?solvhealth\\.com/", 22, [3604], [{ e: 4646 }], [{ v: 3604 }], [{ c: 4647 }, { check: "none", wv: 3604 }], [{ check: "none", v: 3604 }], {}], [1, "sostereo.com", 2, "^https://(www\\.)?sostereo\\.com/", 22, [4648], [{ e: 4648 }], [{ v: 4648 }], [{ c: 4649 }, { w: 4650 }, { all: true, optional: true, k: 4651 }, { c: 4652 }], [], {}], [1, "speciesplus.net", 2, "^https?://(www\\.)?speciesplus\\.net/", 22, [4653], [{ e: 4654 }], [{ v: 4653 }], [{ c: 4655 }, { check: "none", wv: 4653 }], [{ cc: 4656 }], {}], [1, "spermidinelife-us", 1, "^https://(www\\.)?spermidinelife\\.us/", 22, [4657], [{ e: 4658 }], [{ v: 4659 }], [{ h: 4657 }], [], {}], [3, "sportmedizin-rheuma", 1, "^https?://(www\\.)?sportmedizin-rheuma\\.de/", 22, [531], [{ e: 533 }], [{ v: 533 }], [{ h: 4660 }, { stylesheet: "body#home { overflow: visible !important; }" }], [], {}], [1, "sportsmans.com", 2, "^https?://(www\\.)?sportsmans\\.com/", 22, [4661], [{ e: 4661 }, { exists: [".dg-consent-banner", "button.dg-button.reject_all"] }], [{ check: "any", v: 4661 }], [{ waitForThenClick: [".dg-consent-banner", "button.dg-button.reject_all"] }, { h: 4661 }], [], {}], [1, "store2000.fr", 2, "^https?://(www\\.)?store2000\\.fr/", 22, [4662], [{ e: 4662 }], [{ v: 4662 }], [{ c: 4663 }, { h: 4662 }], [], {}], [1, "strato.de", 2, "^https://www\\.strato\\.de/", 22, [4664], [{ e: 4665 }], [{ v: 4665 }], [{ k: 4666 }, { c: 4667 }], [], {}], [1, "studentapan", 2, "^https?://(www\\.)?studentapan\\.se/", 22, [4668], [{ e: 4668 }], [{ v: 4668 }], [{ c: 4669 }, { wv: 4670 }, { all: true, optional: true, k: 4671 }, { c: 4672 }], [{ cc: 4673 }], {}], [2, "stuvia", 1, "^https?://(\\w+\\.)?stuvia\\.com/", 22, [4674], [{ e: 4675 }], [{ v: 4674 }], [{ h: 4674 }, { addStyle: "display: none !important", selector: "#cookiesPermission" }], [], {}], [1, "sujetdebac.fr", 2, "^https?://(www\\.)?sujetdebac\\.fr/", 22, [4676], [{ e: 4676 }], [{ v: 4676 }], [{ c: 4677 }], [{ check: "none", v: 4676 }], {}], [1, "surfshark", 2, "^https?://(\\w+\\.)?surfshark\\.com/", 22, [4678], [{ e: 4678 }], [{ v: 4678 }], [{ if: { e: 4679 }, then: [{ k: 4679 }], else: [{ k: 4680 }, { c: 4681 }] }], [{ cc: 4682 }], {}], [1, "svt.se", 2, "^https://(\\w+\\.)?svt\\.se/", 22, [4683], [{ e: 4683 }], [{ v: 4684 }], [{ c: 4685 }], [{ cc: 4686 }], {}], [1, "synergy-net-au", 2, "^https?://(www\\.)?synergy\\.net\\.au/", 22, [4687], [{ e: 4688 }], [{ v: 4689 }], [{ c: 4690 }], [{ cc: 4691 }], {}], [1, "tcl.com", 2, "^https?://(www\\.)?tcl\\.com/", 22, [4692], [{ e: 4693 }], [{ v: 4693 }], [{ if: { e: 4694 }, then: [{ retry: 5, c: 4694 }], else: [{ retry: 5, c: 4695 }] }], [{ timeout: 2e3, check: "none", wv: 4693 }], {}], [1, "teacherspensions.co.uk", 2, "^https?://(www\\.)?teacherspensions\\.co\\.uk/", 10, [4696], [{ e: 4697 }], [{ v: 4696 }], [{ c: 4697 }, { wv: 4698 }, { all: true, optional: true, k: 4699 }, { k: 4698 }], [{ cc: 4700 }], {}], [1, "teeshoppen", 2, "^https?://(www\\.)?teeshoppen\\.(se|dk|no|fi|de|nl|com|co\\.uk|fr|at)/", 22, [4701], [{ e: 4702 }], [{ v: 4703 }], [{ retry: 3, retryInterval: 1e3, c: 4702 }, { check: "none", wv: 4701 }], [{ cc: 4704 }, { cc: 4705 }], {}], [1, "telegram-im", 2, "^https?://(www\\.)?(telegram\\.im|telegramcn\\.org\\.cn|telegramim\\.ru)/", 22, [4706], [{ e: 4707 }], [{ v: 4706 }], [{ c: 4706 }], [{ check: "none", v: 4706 }], {}], [1, "temu", 2, "^https://([a-z0-9-]+\\.)?temu\\.com/", 22, [], [{ e: 4708 }], [{ v: 4708 }], [{ c: 4709 }], [{ wait: 500 }, { cc: 4710 }], {}], [1, "tesco", 0, "^https://(www\\.)?tesco\\.com/", 22, [4711], [{ e: 4711 }], [{ v: 4711 }], [{ w: 2410 }, { c: 4712 }], [{ cc: 4713 }], {}], [1, "tesla", 2, "^https://(www\\.)?tesla\\.com/", 10, [], [{ e: 300 }], [{ v: 300 }], [{ c: 4714 }], [{ cc: 4715 }], {}], [1, "texas.gov", 2, "^https?://(www\\.)?texas\\.gov/", 22, [4716], [{ e: 4717 }], [{ check: "any", v: 4718 }], [{ if: { v: 4719 }, then: [{ k: 4719 }], else: [{ k: 4720 }] }, { check: "none", wv: 4716 }], [], {}], [1, "thecode", 2, "^https?://(www\\.)?(thecode\\.co\\.uk|lovethesales\\.com)/", 22, [4721], [{ e: 4721 }], [{ v: 4722 }], [{ c: 4723 }, { wv: 4724 }, { if: { e: 4725 }, then: [{ k: 4726 }] }, { k: 4727 }], [{ wait: 1e3 }, { cc: 4728 }], {}], [1, "thegrandyork", 2, "^https?://(www\\.)?thegrandyork\\.co\\.uk/", 22, [4729], [{ e: 4730 }], [{ v: 4731 }], [{ c: 4730 }, { wv: 4732 }, { all: true, optional: true, k: 4733 }, { k: 4734 }], [{ wait: 500 }, { cc: 4735 }], {}], [1, "theinfatuation", 2, "^https://(www\\.)?theinfatuation\\.com", 10, [3486], [{ e: 4736 }], [{ v: 4736 }], [{ c: 4736 }], [{ check: "none", v: 3486 }], {}], [1, "thenounproject", 2, "^https?://(\\w+\\.)?thenounproject\\.com/", 22, [4737], [{ e: 4738 }], [{ check: "any", v: 4738 }], [{ c: 4738 }, { any: [{ timeout: 2e3, check: "any", wv: 4739 }, { k: 4738 }] }, { check: "any", wv: 4739 }, { all: true, optional: true, k: 4740 }, { c: 4741 }], [{ cc: 858 }, { cc: 4742 }], {}], [1, "thenude", 0, "^https?://(www\\.)?thenude\\.com/", 10, [4743], [{ e: 4743 }], [{ v: 4743 }], [{ c: 4744 }, { wv: 4745 }, { if: { e: 4746 }, then: [{ k: 4746 }, { timeout: 1e3, c: 4747 }] }, { if: { e: 4746 }, then: [{ k: 4746 }, { timeout: 1e3, c: 4747 }] }, { if: { e: 4746 }, then: [{ k: 4746 }, { timeout: 1e3, c: 4747 }] }, { if: { e: 4746 }, then: [{ k: 4746 }, { timeout: 1e3, c: 4747 }] }, { k: 4748 }, { check: "none", timeout: 3e3, wv: 4743 }], [{ cc: 2414 }, { cc: 4749 }], {}], [1, "therealest", 2, "^https?://([a-z0-9-]+\\.)?therealest\\.com/", 22, [4750], [{ e: 4750 }], [{ v: 4750 }], [{ c: 4751 }, { c: 4752 }, { w: 4753 }, { c: 4754 }], [{ cc: 858 }], {}], [1, "thomsonlocal.com", 2, "^https?://(www\\.)?thomsonlocal\\.com/", 22, [3441], [{ e: 4755 }], [{ v: 4756 }], [{ c: 4755 }], [{ cc: 4757 }], {}], [1, "tinder", 2, "^https?://(www\\.)?tinder\\.com/", 22, [4758], [{ e: 4758 }], [{ v: 4758 }], [{ c: 4759 }, { c: 4760 }], [{ check: "none", timeout: 2e3, wv: 4758 }], {}], [1, "tineye", 1, "^https://(www\\.)?tineye\\.com/", 22, [4761], [{ e: 4761 }], [{ v: 4761 }], [{ h: 4761 }], [{ check: "none", v: 4761 }], {}], [1, "tinyurl", 1, "^https://tinyurl\\.com/", 22, [4762], [{ e: 4762 }], [{ v: 4762 }], [{ h: 4762 }], [], {}], [1, "tiresize.com", 0, "^https?://(www\\.)?tiresize\\.com/", 22, [4763], [{ e: 4764 }], [{ v: 4763 }], [{ c: 4764 }, { check: "none", timeout: 2e3, wv: 4763 }], [{ cc: 4765 }], {}], [1, "tk-aerztefuehrer.de", 0, "^https?://(www\\.)?tk-aerztefuehrer\\.de/", 10, [4766], [{ e: 4766 }], [{ v: 4766 }], [{ c: 4767 }], [{ cc: 4768 }], {}], [1, "tlc-direct", 2, "^https?://(\\w+\\.)?tlc-direct\\.co\\.uk/", 22, [550], [{ e: 4769 }, { e: 4770 }], [{ v: 550 }], [{ c: 4771 }, { if: { e: 4772 }, then: [{ k: 4773 }] }, { if: { e: 3419 }, then: [{ k: 3420 }] }, { c: 4774 }], [{ cc: 4775 }], {}], [1, "todostuslibros", 2, "^https?://(www\\.)?todostuslibros\\.com/", 22, [4776], [{ e: 4777 }], [{ v: 4776 }], [{ c: 4778 }, { wv: 4777 }, { if: { e: 4779 }, then: [{ k: 4780 }] }, { if: { e: 4781 }, then: [{ k: 4782 }] }, { k: 4777 }], [{ cc: 4783 }, { negated: true, cc: 4784 }], {}], [1, "toho-one.com", 2, "^https?://www\\.toho-one\\.com/", 22, [], [{ e: 4785 }], [{ v: 4785 }], [{ c: 4786 }, { waitFor: ["xpath///div[contains(@class, 'fixed') and .//button[normalize-space()='\u8A2D\u5B9A\u3092\u4FDD\u5B58\u3059\u308B']]", "input[type=checkbox]:checked:not(:disabled)"] }, { click: ["xpath///div[contains(@class, 'fixed') and .//button[normalize-space()='\u8A2D\u5B9A\u3092\u4FDD\u5B58\u3059\u308B']]", "input[type=checkbox]:checked:not(:disabled)"], all: true }, { c: 4787 }], [{ timeout: 1e3, check: "none", wv: 4785 }], {}], [1, "tohotheater-jp", 2, "^https?://(?:[^/]+\\.)?tohotheater\\.jp/", 22, [4788], [{ e: 4789 }], [{ v: 4788 }], [{ c: 4789 }, { c: 4790 }, { c: 4791 }, { c: 4792 }, { c: 4793 }, { c: 4794 }], [{ cc: 4795 }, { cc: 4796 }], {}], [1, "track.amazon.com", 2, "^https://[a-z]*\\.amazon\\.", 22, [4797], [{ e: 4798 }], [{ v: 4798 }], [{ k: 4799 }], [], {}], [1, "tradecalculator.co.uk", 2, "^https?://(www\\.)?tradecalculator\\.co\\.uk/", 22, [], [{ e: 4800 }], [{ v: 4800 }], [{ if: { e: 4801 }, then: [{ k: 4801 }, { timeout: 1e3, w: 4802 }] }, { c: 4803 }], [{ check: "none", timeout: 1e3, wv: 4800 }], {}], [1, "transip-nl", 2, "^https://www\\.transip\\.nl/", 22, [4804], [{ any: [{ e: 4804 }, { e: 4805 }] }], [{ any: [{ v: 4804 }, { v: 4805 }] }], [{ if: { e: 4805 }, then: [{ k: 4806 }], else: [{ k: 4807 }] }], [], {}], [1, "truecar", 1, "^https://(www\\.)?truecar\\.com/", 22, [4808], [{ e: 4809 }], [{ v: 4809 }], [{ h: 4808 }], [], {}], [1, "tu-darmstadt", 2, "^https?://([\\w-]+\\.)*tu-darmstadt\\.de/", 22, [], [{ e: 4810 }], [{ v: 4811 }], [{ if: { e: 4812 }, then: [{ k: 4813 }] }, { if: { e: 4814 }, then: [{ k: 4815 }] }, { retry: 3, retryInterval: 500, c: 4811 }], [{ cc: 1309 }, { negated: true, cc: 4816 }], {}], [1, "tullys-jp", 1, "^https?://(\\w+\\.)?tullys\\.co\\.jp/", 22, [4817], [{ e: 4818 }], [{ v: 4817 }], [{ h: 4817 }], [{ check: "none", v: 4817 }], {}], [1, "turkishairlines.com", 2, "^https?://(\\w+\\.)?turkishairlines\\.com/", 22, [4819], [{ e: 4820 }], [{ v: 4821 }], [{ c: 4821 }], [{ cc: 4822 }], {}], [1, "tusclasesparticulares.com", 2, "^https?://(www\\.)?tusclasesparticulares\\.com/", 22, [4823], [{ e: 4823 }], [{ v: 4823 }], [{ retry: 5, retryInterval: 500, c: 4824 }, { check: "none", timeout: 2e3, wv: 4823 }], [{ cc: 4825 }], {}], [1, "tutosurexcel", 2, "^https?://(\\w+\\.)?tutosurexcel\\.com/", 22, [4826], [{ e: 4826 }], [{ v: 4826 }], [{ c: 4827 }, { wv: 4828 }, { optional: true, k: 4829 }, { k: 4830 }], [{ check: "none", timeout: 2e3, wv: 4826 }], {}], [1, "tvp", 0, "^https?://(\\w+\\.)?tvp\\.pl/", 10, [4831], [{ e: 4832 }], [{ v: 4832 }], [{ c: 4833 }, { w: 4834 }, { if: { e: 4835 }, then: [{ k: 4836 }] }, { k: 4834 }, { check: "none", timeout: 2e3, optional: true, wv: 4832 }], [{ cc: 4837 }], {}], [1, "twitch.tv", 2, "^https?://([\\w-]+\\.)?twitch\\.tv/", 22, [4838], [{ e: 4839 }], [{ v: 4839 }], [{ if: { e: 4840 }, then: [{ retry: 3, retryInterval: 1e3, c: 4841 }, { check: "none", wv: 4839 }], else: [{ c: 4842 }, { w: 4843 }, { all: true, optional: true, k: 4844 }, { c: 4845 }, { check: "none", wv: 4846 }] }], [], {}], [1, "ub-hu-berlin", 2, "^https?://(www\\.)?ub\\.hu-berlin\\.de/", 22, [], [{ e: 4847 }], [{ v: 4848 }], [{ c: 4849 }], [{ cc: 4850 }], {}], [1, "ubisoft.com", 2, "^https?://(\\w+\\.)?ubisoft\\.com/", 22, [3773, 4851], [{ e: 4852 }], [{ check: "any", v: 4853 }], [{ if: { e: 4854 }, then: [{ c: 4854 }], else: [{ c: 4855 }, { c: 4856 }] }], [{ cc: 4857 }, { cc: 4858 }], {}], [1, "udg.edu", 0, "^https?://(\\w+\\.)?udg\\.edu/", 22, [3324], [{ e: 4859 }], [{ v: 3324 }], [{ c: 4860 }, { wv: 4861 }, { all: true, optional: true, k: 4862 }, { retry: 3, retryInterval: 500, c: 4863 }], [{ cc: 4864 }], {}], [1, "uhcprovider", 2, "^https?://(www\\.)?uhc(provider|dental)\\.com/", 22, [4865], [{ e: 4866 }], [{ v: 4865 }], [{ c: 4867 }], [{ check: "none", timeout: 2e3, wv: 4865 }], {}], [1, "unicourt", 1, "^https://(www\\.)?unicourt\\.com/", 22, [3456], [{ e: 3456 }], [{ v: 3456 }], [{ h: 3456 }], [], {}], [1, "unitedconsumers.com", 2, "^https?://(www\\.)?unitedconsumers\\.com/", 22, [4868], [{ e: 4869 }], [{ v: 4868 }], [{ retry: 10, retryInterval: 500, c: 4870 }, { check: "none", wv: 4868 }], [{ cc: 4871 }], {}], [1, "unitedutilities", 2, "^https?://(www\\.)?unitedutilities\\.com/", 10, [4872], [{ e: 4873 }], [{ v: 4874 }], [{ c: 4873 }, { wv: 4875 }, { optional: true, k: 4876 }, { optional: true, k: 4877 }, { k: 4875 }], [{ cc: 4878 }, { cc: 4879 }], {}], [1, "unive-nl", 0, "^https?://(?:[^/]+\\.)?unive\\.nl/", 10, [4880], [{ e: 4881 }], [{ v: 4880 }], [{ c: 4882 }], [{ check: "none", timeout: 2e3, wv: 4880 }], {}], [1, "unive.it", 0, "^https?://(\\w+\\.)?unive\\.it/", 22, [4883], [{ e: 4884 }], [{ any: [{ v: 4884 }, { negated: true, cc: 4885 }] }], [{ timeout: 15e3, wv: 4884 }, { retry: 3, retryInterval: 500, c: 4884 }], [{ cc: 4885 }, { negated: true, cc: 4886 }, { negated: true, cc: 4887 }], {}], [1, "uregina", 1, "^https?://(\\w+\\.)?uregina\\.ca/", 22, [4888], [{ e: 4889 }], [{ v: 4888 }], [{ h: 4888 }], [{ check: "none", v: 4888 }], {}], [1, "urvaerket", 2, "^https?://(www\\.)?(urvaerket\\.dk|uret\\.se|urverket\\.no|laatukellot\\.fi|thewatchagency\\.com)/", 22, [4890], [{ e: 4891 }], [{ v: 4891 }], [{ c: 4892 }], [{ cc: 4775 }], {}], [1, "usherbrooke", 2, "^https?://(\\w+\\.)?usherbrooke\\.ca/", 22, [4893], [{ e: 4894 }], [{ v: 312 }], [{ c: 4895 }, { wv: 4896 }, { if: { e: 3419 }, then: [{ k: 3420 }] }, { if: { e: 4897 }, then: [{ k: 4898 }] }, { retry: 3, retryInterval: 600, c: 4896 }, { check: "none", wv: 4899 }], [], {}], [1, "uswitch.com", 2, "^https://(www\\.)?uswitch\\.com/", 10, [4900], [{ e: 4901 }], [{ v: 4901 }], [{ c: 4902 }], [], {}], [1, "uttings.co.uk", 2, "^https?://(www\\.)?uttings\\.co\\.uk/", 22, [929, 4903], [{ e: 4904 }], [{ v: 4904 }], [{ c: 4904 }, { waitForVisible: ["#colorbox iframe.cboxIframe", "#cookie_notice_save"], timeout: 1e4 }, { click: ["#colorbox iframe.cboxIframe", ".cookie-field input[type=checkbox]:checked:not(:disabled)"], all: true, optional: true }, { waitForThenClick: ["#colorbox iframe.cboxIframe", "#cookie_notice_save"], timeout: 1e4, retry: 5, retryInterval: 1e3 }], [{ cc: 4905 }], {}], [1, "uwv-nl", 0, "^https?://(?:www\\.)?uwv\\.nl/", 10, [4906], [{ e: 4906 }], [{ v: 4907 }], [{ c: 4907 }], [{ check: "none", timeout: 2e3, wv: 4907 }], {}], [1, "vantilburgonline", 2, "^https?://(www\\.)?vantilburgonline\\.nl/", 22, [], [{ e: 4908 }], [{ v: 4908 }], [{ c: 4909 }, { c: 4910 }, { c: 4911 }], [{ cc: 4912 }], {}], [1, "vd-ch", 0, "^https?://(\\w+\\.)?vd\\.ch/", 22, [], [{ e: 4913 }], [{ v: 4913 }], [{ c: 4913 }], [{ cc: 4914 }], {}], [1, "venngage.com", 2, "^https?://(\\w+\\.)?venngage\\.com/", 22, [4915, 4916], [{ e: 4917 }, { v: 4918 }], [{ v: 4918 }], [{ c: 4917 }, { c: 4919 }], [{ cc: 4920 }], {}], [1, "vgn.de", 2, "^https?://(\\w+\\.)?vgn\\.de/", 22, [4921], [{ e: 4922 }], [{ v: 4923 }], [{ if: { e: 4924 }, then: [{ all: true, k: 4924 }] }, { c: 4922 }, { check: "none", timeout: 2e3, wv: 4923 }], [{ cc: 4925 }, { negated: true, cc: 4926 }], {}], [1, "vinnies-org-au", 1, "^https?://(\\w+\\.)?vinnies\\.org\\.au/", 22, [4927], [{ e: 4928 }], [{ v: 4927 }], [{ h: 4927 }], [], {}], [1, "vinnova-se", 2, "^https?://(www\\.)?vinnova\\.se/", 22, [4929], [{ e: 4930 }], [{ v: 4930 }], [{ c: 4931 }, { wv: 4932 }, { if: { e: 4933 }, then: [{ k: 4933 }] }, { c: 4934 }], [{ cc: 4935 }], {}], [1, "vmock", 2, "^https?://(\\w+\\.)?vmock\\.com/", 22, [4936, 4937], [{ e: 4938 }], [{ v: 4938 }], [{ c: 4938 }, { c: 4939 }, { timeout: 2e3, check: "none", wv: 4937 }], [{ cc: 2520 }], {}], [1, "vodafone.de", 2, "^https://www\\.vodafone\\.de/", 22, [4940], [{ e: 4941 }], [{ v: 4942 }], [{ k: 4943 }], [], {}], [1, "vogue.nl", 2, "^https?://(\\w+\\.)?vogue\\.nl/", 22, [4944], [{ e: 4945 }], [{ v: 4946 }], [{ if: { e: 4947 }, then: [{ k: 4947 }] }, { c: 4948 }], [{ cc: 4949 }], {}], [1, "volvocars-com", 2, "^https://([a-z0-9-]+\\.)?volvocars\\.com/", 10, [4950], [{ exists: ["cookie-banner#cookie-banner-host", "#dialog_cookie_consent"] }], [{ visible: ["cookie-banner#cookie-banner-host", "#dialog_cookie_consent"] }], [{ wait: 500 }, { if: { exists: ["cookie-banner#cookie-banner-host", "#onetrust-reject-all-handler"] }, then: [{ waitForThenClick: ["cookie-banner#cookie-banner-host", "#onetrust-reject-all-handler"] }], else: [{ waitForThenClick: ["cookie-banner#cookie-banner-host", "#onetrust-pc-btn-handler"] }, { c: 2411 }] }, { waitForVisible: ["cookie-banner#cookie-banner-host", "#dialog_cookie_consent"], check: "none", timeout: 5e3 }], [{ cc: 2414 }, { cc: 4951 }, { cc: 4952 }], {}], [1, "vowels", 1, "^https://(www\\.)?vowels\\.(net|jp)/", 22, [4953], [{ e: 4954 }], [{ v: 4955 }], [{ h: 4953 }], [], {}], [1, "vszutphen.nl", 2, "^https?://(www\\.)?vszutphen\\.nl/", 22, [4956], [{ e: 4956 }], [{ v: 4956 }], [{ c: 4957 }], [{ check: "none", timeout: 2e3, wv: 4956 }], {}], [1, "walmart-ca", 0, "^https?://(www\\.)?walmart\\.ca/", 22, [], [{ e: 4958 }], [{ v: 4958 }], [{ retry: 5, retryInterval: 500, c: 4959 }, { c: 4960 }], [{ check: "none", timeout: 5e3, wv: 4958 }], {}], [1, "warszawa19115.pl", 2, "^https?://(\\w+\\.)?warszawa19115\\.pl/", 22, [], [{ e: 4961 }], [{ v: 4961 }], [{ c: 4962 }, { wv: 4963 }, { if: { e: 4964 }, then: [{ all: true, k: 4964 }] }, { k: 4963 }], [{ check: "none", timeout: 3e3, wv: 4965 }], {}], [1, "web-rewind.com", 0, "^https?://(www\\.)?web-rewind\\.com/", 22, [], [{ e: 4966 }], [{ v: 4966 }], [{ all: true, timeout: 500, optional: true, c: 4967 }, { c: 4966 }], [], {}], [1, "westminster.ac.uk", 2, "^https?://(\\w+\\.)?westminster\\.ac\\.uk/", 22, [4968], [{ e: 4969 }], [{ v: 4970 }], [{ c: 4971 }, { wv: 4972 }, { all: true, optional: true, k: 4973 }, { k: 4972 }], [{ cc: 4974 }, { cc: 4975 }], {}], [1, "what3words", 2, "^https?://(\\w+\\.)?what3words\\.com/", 22, [], [{ e: 4976 }], [{ check: "any", v: 4976 }], [{ if: { e: 4977 }, then: [{ k: 4977 }, { c: 4978 }], else: [{ k: 4979 }, { c: 4980 }] }, { timeout: 4e3, optional: true, wv: 2802 }, { if: { v: 2802 }, then: [{ k: 1171 }, { all: true, optional: true, k: 1173 }, { optional: true, k: 1174 }] }], [{ any: [{ cc: 4775 }, { cc: 4981 }] }], {}], [1, "wikiwand", 1, "^https://(www\\.)?wikiwand\\.com/", 22, [4982], [{ e: 4982 }], [{ v: 4982 }], [{ h: 4982 }], [], {}], [1, "wildling-com", 1, "^https://(www\\.)?wildling\\.com/", 22, [4983], [{ e: 4984 }], [{ v: 4984 }], [{ h: 4983 }], [], {}], [1, "wojsko-polskie-pl", 1, "^https?://(www\\.)?wojsko-polskie\\.pl/", 22, [4985], [{ e: 4986 }], [{ v: 4985 }], [{ h: 4985 }], [], {}], [1, "xe.com", 2, "^https?://(www\\.)?xe\\.com/", 22, [4987, 4988], [{ e: 4989 }], [{ v: 4989 }], [{ retry: 3, retryInterval: 500, c: 4990 }, { wv: 4991 }, { c: 4992 }], [{ cc: 4993 }], {}], [1, "xhamster-eu", 2, "^https://(\\w+\\.)?xhamster\\d?\\.com", 22, [4994], [{ e: 4995 }], [{ v: 4995 }], [{ wv: 4996 }, { wait: 500 }, { k: 4996 }, { optional: true, h: 4997 }], [], {}], [1, "xhamster-us", 1, "^https://(\\w+\\.)?xhamster\\d?\\.com", 22, [4998], [{ e: 4998 }], [{ v: 4999 }], [{ h: 4998 }], [], {}], [1, "xlbygg.se", 2, "^https?://(\\w+\\.)?xlbygg\\.se/", 22, [], [{ e: 5e3 }], [{ v: 5001 }], [{ c: 5002 }, { wv: 5003 }, { all: true, optional: true, k: 5004 }, { k: 5e3 }], [{ cc: 5005 }], {}], [1, "xtb", 2, "^https?://(www\\.)?xtb\\.com/", 22, [5006, 4392], [{ e: 5007 }], [{ v: 5006 }], [{ c: 5008 }, { wv: 5009 }, { all: true, optional: true, k: 5010 }, { k: 5009 }], [{ check: "none", wv: 5006 }, { cc: 5011 }], {}], [1, "xvideos", 2, "^https://[a-z]*\\.xvideos\\.com/", 22, [], [{ e: 2685 }], [{ v: 2686 }], [{ c: 2687 }], [], {}], [1, "yachtclubgames.com", 2, "^https?://(www\\.)?yachtclubgames\\.com/", 22, [5012], [{ e: 5012 }], [{ v: 5012 }], [{ c: 5013 }], [{ timeout: 1e3, check: "none", wv: 5012 }], {}], [1, "Yahoo", 2, "^https://consent\\.yahoo\\.com/v2/", 22, [2314], [{ e: 5014 }], [{ v: 5014 }], [{ c: 5015 }], [], {}], [1, "ynjn", 2, "^https?://(www\\.)?ynjn\\.jp/", 22, [5016], [{ e: 5016 }], [{ v: 5017 }], [{ c: 5018 }], [{ timeout: 1e3, check: "none", wv: 5017 }], {}], [1, "yourspares", 2, "^https?://(www\\.)?yourspares\\.co\\.uk/", 22, [5019, 5020], [{ e: 5021 }], [{ v: 5019 }], [{ w: 5022 }, { k: 5022 }, { check: "none", wv: 5019 }], [{ cc: 5023 }], {}], [1, "yuntrack", 2, "^https://(www\\.|m\\.)?yuntrack\\.com/", 22, [3358], [{ e: 5024 }], [{ v: 3358 }], [{ c: 5024 }, { wv: 5025 }, { if: { e: 5026 }, then: [{ all: true, k: 5026 }] }, { c: 5027 }], [{ cc: 5028 }], {}], [1, "zentralruf-de", 2, "^https://(www\\.)?zentralruf\\.de", 22, [5029], [{ e: 5029 }], [{ v: 5029 }], [{ c: 5030 }], [], {}], [1, "zinio", 0, "^https://(www\\.)?zinio\\.com/", 22, [], [{ e: 5031 }], [{ v: 5031 }], [{ c: 5032 }, { c: 5033 }], [{ cc: 5034 }], {}], [1, "ziptozipmoving", 2, "^https://(www\\.)?ziptozipmoving\\.com/", 22, [], [{ e: 5035 }], [{ v: 5035 }], [{ c: 5036 }, { wv: 5037 }, { all: true, optional: true, k: 5038 }, { c: 5039 }], [{ cc: 5040 }], {}], [1, "zoomalia", 2, "^https?://(www\\.)?zoomalia\\.\\w+/", 22, [], [{ e: 5041 }], [{ v: 5041 }], [{ c: 5042 }], [{ cc: 5043 }, { cc: 5044 }], {}], [1, "zorgenzekerheid.nl", 2, "^https?://(www\\.)?zorgenzekerheid\\.nl/", 22, [5045], [{ exists: ["zz-cookie-banner", "zz-drawer"] }], [{ visible: ["zz-cookie-banner[visible]", "zz-drawer"] }], [{ waitForThenClick: ["zz-cookie-banner", ".footer zz-button[variant=ghost]", "button"] }, { waitForVisible: ["zz-cookie-banner[visible]", "zz-drawer"], check: "none" }], [{ cc: 5046 }, { cc: 5047 }], {}]], index: { genericRuleRange: [0, 602], frameRuleRange: [600, 635], specificRuleRange: [602, 1571], genericStringEnd: 2721, frameStringEnd: 2789 } };

  // standalone/content.ts
  function isMainFrame() {
    return window.top === window;
  }
  function buildRules() {
    return {
      autoconsent: [],
      compact: filterCompactRules(compact_rules_default, {
        url: window.location.href,
        mainFrame: isMainFrame()
      })
    };
  }
  function logMessage(message) {
    switch (message.type) {
      case "cmpDetected":
        console.log(`autoconsent detected CMP: ${message.cmp}`);
        break;
      case "popupFound":
        console.log(`autoconsent found popup: ${message.cmp}`);
        break;
      case "optOutResult":
        console.log(`autoconsent opt-out result: ${message.cmp}`, message.result);
        break;
      case "autoconsentDone":
        console.log(`autoconsent done: ${message.cmp}`, {
          duration: message.duration,
          totalClicks: message.totalClicks
        });
        break;
      case "autoconsentError":
        console.warn("autoconsent error", message.details);
        break;
    }
  }
  if (!window.autoconsentReceiveMessage) {
    const config = {
      isMainWorld: true,
      enableHeuristicDetection: true,
      heuristicMode: "tier2",
      enablePopupMutationObserver: false,
      logs: {
        lifecycle: true,
        rulesteps: true,
        detectionsteps: false,
        evals: false,
        errors: true,
        messages: false,
        waits: true
      }
    };
    const rules = buildRules();
    const messages = [];
    const consentRef = {};
    window.autoconsentReceiveMessage = async (message) => {
      if (!consentRef.current) {
        throw new Error("autoconsent is not initialized yet");
      }
      await consentRef.current.receiveMessageCallback(message);
    };
    const sendMessage = async (message) => {
      messages.push(message);
      logMessage(message);
    };
    const consent = new AutoConsent(sendMessage);
    consentRef.current = consent;
    window.autoconsentStandalone = {
      instance: consent,
      messages
    };
    consent.initialize(config, rules);
    console.log("autoconsent standalone initialized", {
      instanceId: consent.id,
      rules: rules.compact?.r.length ?? 0,
      url: window.location.href
    });
  } else {
    console.warn("autoconsent already initialized", window.autoconsentReceiveMessage);
  }
})();
