(function () {
  var root = document.documentElement;
  var buttons = Array.prototype.slice.call(document.querySelectorAll('[data-set-lang]'));
  var hero = document.querySelector('.hero-shot img');
  var titles = {
    en: 'Synector - Windows automation suite and process supervisor',
    ru: 'Synector - набор автоматизации для Windows'
  };

  function apply(lang, persist) {
    root.dataset.lang = lang;
    root.lang = lang;
    document.title = titles[lang] || titles.en;

    buttons.forEach(function (button) {
      button.setAttribute('aria-pressed', String(button.dataset.setLang === lang));
    });

    if (hero) {
      var alt = hero.getAttribute('data-alt-' + lang);
      if (alt) {
        hero.alt = alt;
      }
    }

    if (persist) {
      try {
        localStorage.setItem('synector-lang', lang);
      } catch (error) {}
    }
  }

  buttons.forEach(function (button) {
    button.addEventListener('click', function () {
      apply(button.dataset.setLang, true);
    });
  });

  apply(root.dataset.lang === 'ru' ? 'ru' : 'en', false);

  var copyButtons = Array.prototype.slice.call(document.querySelectorAll('[data-copy]'));

  function markCopied(button) {
    button.classList.add('is-copied');
    clearTimeout(button.copyTimer);
    button.copyTimer = setTimeout(function () {
      button.classList.remove('is-copied');
    }, 1800);
  }

  function fallbackCopy(text) {
    var field = document.createElement('textarea');
    field.value = text;
    field.setAttribute('readonly', '');
    field.style.position = 'fixed';
    field.style.opacity = '0';
    document.body.appendChild(field);
    field.select();
    try {
      document.execCommand('copy');
    } catch (error) {}
    document.body.removeChild(field);
  }

  copyButtons.forEach(function (button) {
    button.addEventListener('click', function () {
      var text = button.getAttribute('data-copy') || '';
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
