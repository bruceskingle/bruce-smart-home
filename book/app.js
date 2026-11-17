// Offers the app's two themes, see app.css: Light, and Dark, which is mdBook's Navy theme restyled.
(function () {
    for (const name of ["rust", "coal", "ayu"]) {
        const button = document.getElementById("mdbook-theme-" + name);
        if (button && button.parentElement) {
            button.parentElement.style.display = "none";
        }
    }
    const dark = document.getElementById("mdbook-theme-navy");
    if (dark) {
        dark.textContent = "Dark";
    }
})();

// Opens a picture, e.g. a screenshot, full size in a new tab when it is clicked, as GitHub does, apart from the icons
// of the app's buttons in the help, see app.css.
(function () {
    // mdBook repeats each picture for its zoom, see app.css, which is not used.
    for (const image of document.querySelectorAll(".content img:not(.img-wrapper img)")) {
        if (image.closest("a") || image.getAttribute("src").includes("icons/")) {
            continue;
        }
        image.classList.add("full-size-link");
        image.title = image.title || "Click to see it full size";
        image.addEventListener("click", (event) => {
            // Rather than also zooming it on the page, which mdBook does when a picture is clicked.
            event.preventDefault();
            window.open(image.src, "_blank", "noopener");
        });
    }
})();
