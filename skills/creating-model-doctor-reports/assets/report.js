(() => {
  "use strict";

  document.querySelectorAll(".result-row").forEach((row) => {
    row.addEventListener("click", () => {
      const detail = document.getElementById(row.dataset.detailId);
      const button = row.querySelector(".row-toggle");
      if (!detail || !button) return;

      const expanded = button.getAttribute("aria-expanded") !== "true";
      button.setAttribute("aria-expanded", expanded ? "true" : "false");
      button.setAttribute(
        "aria-label",
        `${expanded ? "收起" : "展开"}检测项 ${button.getAttribute("aria-controls").replace("test-detail-", "")}`,
      );
      detail.hidden = !expanded;
      row.classList.toggle("is-expanded", expanded);
    });
  });
})();
