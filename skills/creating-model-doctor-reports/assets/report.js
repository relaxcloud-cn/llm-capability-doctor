(() => {
  "use strict";

  let currentFilter = "all";
  const search = document.getElementById("test-search");
  const visibleCount = document.getElementById("visible-count");
  const emptyState = document.getElementById("empty-state");

  const applyFilters = () => {
    if (!search || !visibleCount) return;
    const query = search.value.trim().toLowerCase();
    let count = 0;
    document.querySelectorAll(".domain").forEach((domain) => {
      let domainCount = 0;
      domain.querySelectorAll(".test-row").forEach((row) => {
        const matchesFilter = currentFilter === "all"
          || (currentFilter === "fail" && row.dataset.status === "FAIL")
          || row.dataset.tier === currentFilter;
        const matchesSearch = !query || (row.dataset.search || "").includes(query);
        const visible = matchesFilter && matchesSearch;
        row.hidden = !visible;
        if (visible) { domainCount += 1; count += 1; }
      });
      domain.hidden = domainCount === 0;
      if (query || currentFilter !== "all") domain.open = domainCount > 0;
    });
    visibleCount.textContent = `当前显示 ${count} 个检测项`;
    if (emptyState) emptyState.style.display = count ? "none" : "block";
  };

  document.querySelectorAll(".filter-button").forEach((button) => {
    button.addEventListener("click", () => {
      currentFilter = button.dataset.filter;
      document.querySelectorAll(".filter-button").forEach((item) => item.classList.toggle("is-active", item === button));
      applyFilters();
    });
  });
  if (search) search.addEventListener("input", applyFilters);

  const mobileNav = document.getElementById("mobile-section-nav");
  if (mobileNav) {
    mobileNav.addEventListener("change", (event) => {
      const target = document.getElementById(event.target.value);
      if (target) target.scrollIntoView({ behavior: "smooth" });
    });
  }

  const navLinks = Array.from(document.querySelectorAll(".sidebar-nav a"));
  const sections = document.querySelectorAll("[data-nav-section]");
  if (navLinks.length && sections.length && "IntersectionObserver" in window) {
    const observer = new IntersectionObserver((entries) => {
      const visible = entries
        .filter((entry) => entry.isIntersecting)
        .sort((a, b) => b.intersectionRatio - a.intersectionRatio)[0];
      if (!visible) return;
      navLinks.forEach((link) => link.classList.toggle("is-active", link.getAttribute("href") === `#${visible.target.id}`));
    }, { rootMargin: "-20% 0px -65% 0px", threshold: [0, 0.2, 0.5] });
    sections.forEach((section) => observer.observe(section));
  }
})();
