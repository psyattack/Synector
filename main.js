(function () {
  var root = document.documentElement;
  var langButtons = Array.prototype.slice.call(document.querySelectorAll("[data-set-lang]"));
  var video = document.getElementById("product-demo");
  var navToggle = document.querySelector("[data-nav-toggle]");
  var mobileNav = document.getElementById("mobile-nav");
  var langGroup = document.querySelector(".lang");
  var versionPill = document.querySelector("[data-version-pill]");
  var latestVersion = versionPill ? versionPill.textContent.trim() : "";
  var releaseCacheKey = "synector-latest-release";
  var releaseCacheTtl = 60 * 60 * 1000;
  var titles = {
    en: "Synector - Windows automation suite and process supervisor",
    ru: "Synector - набор автоматизации и супервизор процессов для Windows"
  };
  var descriptions = {
    en: "A native Windows automation suite written in pure Rust. A tray daemon owns rules, triggers and counters; the GUI is an editor you can close. No telemetry.",
    ru: "Нативный набор автоматизации для Windows на чистом Rust. Демон в трее владеет правилами, триггерами и счётчиками, а интерфейс можно закрыть. Без телеметрии."
  };

  function currentLang() {
    return root.dataset.lang === "ru" ? "ru" : "en";
  }

  function setMetaDescription() {
    var meta = document.querySelector('meta[name="description"]');
    if (meta) {
      meta.setAttribute("content", descriptions[currentLang()] || descriptions.en);
    }
  }

  function setVideoLabel() {
    if (!video) {
      return;
    }
    var aria = video.getAttribute("data-label-" + currentLang());
    if (aria) {
      video.setAttribute("aria-label", aria);
    }
  }

  function applyLang(lang, persist) {
    root.dataset.lang = lang;
    root.lang = lang;
    document.title = titles[lang] || titles.en;
    setMetaDescription();
    setVideoLabel();
    setVersionPillLabel();

    if (langGroup) {
      langGroup.setAttribute("aria-label", lang === "ru" ? "Язык" : "Language");
    }

    langButtons.forEach(function (button) {
      button.setAttribute("aria-pressed", String(button.dataset.setLang === lang));
    });

    if (navToggle) {
      var sr = navToggle.querySelector(".sr-only");
      if (sr) {
        sr.textContent = lang === "ru" ? "Меню" : "Menu";
      }
    }

    if (persist) {
      try {
        localStorage.setItem("synector-lang", lang);
      } catch (error) {}
    }
  }

  function setVersionPillLabel() {
    if (!versionPill || !latestVersion) {
      return;
    }
    var russian = currentLang() === "ru";
    versionPill.setAttribute(
      "aria-label",
      russian ? "Synector " + latestVersion + " — последний релиз" : "Synector " + latestVersion + " latest release"
    );
    versionPill.title = russian ? "Открыть последний релиз" : "Open the latest release";
  }

  function applyLatestRelease(tag, url) {
    var version = String(tag || "").trim();
    if (!versionPill || !version) {
      return;
    }
    latestVersion = version;
    versionPill.textContent = version;
    if (url) {
      versionPill.href = url;
    }
    setVersionPillLabel();
  }

  function loadLatestRelease() {
    if (!versionPill || typeof fetch !== "function") {
      return;
    }
    try {
      var cached = JSON.parse(localStorage.getItem(releaseCacheKey) || "null");
      if (cached && cached.tag && Date.now() - cached.at < releaseCacheTtl) {
        applyLatestRelease(cached.tag, cached.url);
        return;
      }
    } catch (error) {}

    fetch("https://api.github.com/repos/psyattack/Synector/releases/latest", {
      headers: { Accept: "application/vnd.github+json" }
    })
      .then(function (response) {
        return response.ok ? response.json() : null;
      })
      .then(function (release) {
        if (!release || !release.tag_name) {
          return;
        }
        applyLatestRelease(release.tag_name, release.html_url);
        try {
          localStorage.setItem(releaseCacheKey, JSON.stringify({
            tag: versionPill.textContent.trim(),
            url: release.html_url || versionPill.href,
            at: Date.now()
          }));
        } catch (error) {}
      })
      .catch(function () {});
  }

  langButtons.forEach(function (button) {
    button.addEventListener("click", function () {
      applyLang(button.dataset.setLang, true);
    });
  });

  applyLang(currentLang(), false);
  loadLatestRelease();

  if (navToggle && mobileNav) {
    navToggle.addEventListener("click", function () {
      var open = navToggle.getAttribute("aria-expanded") !== "true";
      navToggle.setAttribute("aria-expanded", String(open));
      mobileNav.hidden = !open;
    });

    mobileNav.addEventListener("click", function (event) {
      if (event.target.closest("a")) {
        navToggle.setAttribute("aria-expanded", "false");
        mobileNav.hidden = true;
      }
    });

    document.addEventListener("keydown", function (event) {
      if (event.key === "Escape" && navToggle.getAttribute("aria-expanded") === "true") {
        navToggle.setAttribute("aria-expanded", "false");
        mobileNav.hidden = true;
      }
    });

    window.addEventListener("resize", function () {
      if (window.innerWidth > 900 && navToggle.getAttribute("aria-expanded") === "true") {
        navToggle.setAttribute("aria-expanded", "false");
        mobileNav.hidden = true;
      }
    });
  }

  var copyButtons = Array.prototype.slice.call(document.querySelectorAll("[data-copy]"));

  function markCopied(button) {
    button.classList.add("is-copied");
    clearTimeout(button.copyTimer);
    button.copyTimer = setTimeout(function () {
      button.classList.remove("is-copied");
    }, 1800);
  }

  function fallbackCopy(text) {
    var field = document.createElement("textarea");
    field.value = text;
    field.setAttribute("readonly", "");
    field.style.position = "fixed";
    field.style.opacity = "0";
    document.body.appendChild(field);
    field.select();
    try {
      document.execCommand("copy");
    } catch (error) {}
    document.body.removeChild(field);
  }

  copyButtons.forEach(function (button) {
    button.addEventListener("click", function () {
      var text = button.getAttribute("data-copy") || "";
      if (navigator.clipboard && window.isSecureContext) {
        navigator.clipboard.writeText(text).then(function () {
          markCopied(button);
        }).catch(function () {
          fallbackCopy(text);
          markCopied(button);
        });
      } else {
        fallbackCopy(text);
        markCopied(button);
      }
    });
  });
})();
