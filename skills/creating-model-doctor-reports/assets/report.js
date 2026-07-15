(() => {
  "use strict";

  const state = { status: "all", gate: "all", discrepancy: "all" };
  const items = Array.from(document.querySelectorAll(".test-item"));
  const buttons = Array.from(document.querySelectorAll("[data-filter]"));
  const expandButton = document.querySelector("[data-action='expand-all']");

  const applyFilters = () => {
    items.forEach((item) => {
      const visible =
        (state.status === "all" || item.dataset.status === state.status) &&
        (state.gate === "all" || item.dataset.gate === state.gate) &&
        (state.discrepancy === "all" || item.dataset.discrepancy === state.discrepancy);
      item.hidden = !visible;
    });

    document.querySelectorAll(".category-section").forEach((section) => {
      const visibleCount = section.querySelectorAll(".test-item:not([hidden])").length;
      section.hidden = visibleCount === 0;
    });
  };

  buttons.forEach((button) => {
    button.addEventListener("click", () => {
      const group = button.dataset.filter;
      state[group] = button.dataset.value;
      buttons
        .filter((candidate) => candidate.dataset.filter === group)
        .forEach((candidate) => candidate.setAttribute("aria-pressed", candidate === button ? "true" : "false"));
      applyFilters();
    });
  });

  if (expandButton) {
    expandButton.addEventListener("click", () => {
      const visibleItems = items.filter((item) => !item.hidden);
      const shouldOpen = visibleItems.some((item) => !item.open);
      visibleItems.forEach((item) => { item.open = shouldOpen; });
      expandButton.textContent = shouldOpen ? "收起全部证据" : "展开全部证据";
    });
  }
})();
