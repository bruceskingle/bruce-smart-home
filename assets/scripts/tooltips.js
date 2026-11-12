// Shows the tooltip of anything with a title, which main.css underlines with dots, in a popup at once when it is
// clicked, as the browser's own takes a while to appear, which a page cannot change. The next click, anywhere, closes
// it, as does Escape, or scrolling. Not clicks on or in things which do something else when clicked, controls, links,
// and the like, which main.css does not underline either.
(() => {
    if (window.bruceTooltips) {
        return;
    }
    window.bruceTooltips = true;
    const CLICKABLE = 'button, input, select, textarea, a, option, .link, .export-file-name, .help-resize, .log-resize, th.sortable';
    let popup = null;
    let openedBy = null;

    function close() {
        if (popup) {
            popup.remove();
        }
        popup = null;
        openedBy = null;
    }

    // In the capture phase, before anything else, which still gets the click.
    document.addEventListener('click', (event) => {
        const element = event.target.closest(CLICKABLE) ? null : event.target.closest('[title]');
        const again = element !== null && element === openedBy;
        close();
        if (!element || again || !element.title) {
            return;
        }
        popup = document.createElement('div');
        popup.className = 'tooltip-popup';
        popup.textContent = element.title;
        document.body.appendChild(popup);
        openedBy = element;
        // Below it, or above if there is no room, within the window.
        const rect = element.getBoundingClientRect();
        const left = Math.max(8, Math.min(rect.left, window.innerWidth - popup.offsetWidth - 8));
        let top = rect.bottom + 4;
        if (top + popup.offsetHeight > window.innerHeight - 8) {
            top = Math.max(8, rect.top - popup.offsetHeight - 4);
        }
        popup.style.left = `${left}px`;
        popup.style.top = `${top}px`;
    }, true);
    document.addEventListener('keydown', (event) => {
        if (event.key === 'Escape') {
            close();
        }
    });
    window.addEventListener('scroll', close, true);
})();
