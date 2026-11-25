(function () {
  var sidebar = document.querySelector(".docs-sidebar");
  var active = sidebar ? sidebar.querySelector(".docs-side-link.is-active") : null;

  if (sidebar && active && window.matchMedia("(max-width: 900px)").matches) {
    var sidebarLeft = sidebar.getBoundingClientRect().left;
    var activeLeft = active.getBoundingClientRect().left;
    sidebar.scrollLeft = Math.max(0, activeLeft - sidebarLeft + sidebar.scrollLeft - 12);
  }

  var blocks = Array.prototype.slice.call(document.querySelectorAll(".docs-article pre"));

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

  function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) {
      return navigator.clipboard.writeText(text);
    }
    return Promise.reject(new Error("clipboard unavailable"));
  }

  blocks.forEach(function (pre) {
    var host = pre.parentElement;
    var wrap;

    if (host && host.classList.contains("highlight")) {
      wrap = host;
    } else {
      wrap = document.createElement("div");
      wrap.className = "code-wrap";
      pre.parentNode.insertBefore(wrap, pre);
      wrap.appendChild(pre);
    }

    var button = document.createElement("button");
    button.type = "button";
    button.className = "code-copy";
    button.textContent = "Copy";
    button.setAttribute("aria-label", "Copy code");

    button.addEventListener("click", function () {
      copyText(pre.textContent).then(function () {
        button.classList.add("is-copied");
        button.textContent = "Copied";
      }).catch(function () {
        fallbackCopy(pre.textContent);
        button.classList.add("is-copied");
        button.textContent = "Copied";
      }).finally(function () {
        window.setTimeout(function () {
          button.classList.remove("is-copied");
          button.textContent = "Copy";
        }, 1600);
      });
    });

    wrap.appendChild(button);
  });
})();
