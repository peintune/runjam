import { createRouter, createWebHistory } from "vue-router";
import { useAppTabsStore } from "../stores/useAppTabsStore";
import { track } from "../api/telemetry";
import WorkspaceLayout from "../components/WorkspaceLayout.vue";
import SettingsLayout from "../components/SettingsLayout.vue";

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: "/",
      name: "workspace",
      component: WorkspaceLayout,
    },
    {
      path: "/board",
      name: "board",
      component: () => import("../components/WorkspaceLayout.vue"),
    },
    {
      path: "/settings",
      component: SettingsLayout,
      children: [
        { path: "", redirect: "/settings/models" },
        {
          path: "agents",
          name: "settings-agents",
          component: () => import("../views/settings/AgentSettings.vue"),
        },
        {
          path: "agents/:agentId",
          name: "settings-agent-detail",
          component: () => import("../views/settings/AgentDetailPage.vue"),
        },
        {
          path: "apps",
          name: "settings-apps",
          component: () => import("../views/settings/AppsSettings.vue"),
        },
        {
          path: "models/commercial",
          name: "settings-models-commercial",
          component: () => import("../views/settings/CommercialModelsSettings.vue"),
        },
        {
          path: "models",
          name: "settings-models",
          component: () => import("../views/settings/LocalModelsSettings.vue"),
        },
        {
          path: "general",
          name: "settings-general",
          component: () => import("../views/settings/GeneralSettings.vue"),
        },
        {
          path: "costs",
          name: "settings-costs",
          component: () => import("../views/settings/CostsSettings.vue"),
        },
        {
          path: "skills",
          name: "settings-skills",
          component: () => import("../views/settings/SkillsSettings.vue"),
        },
        {
          path: "about",
          name: "settings-about",
          component: () => import("../views/settings/AboutSettings.vue"),
        },
      ],
    },
  ],
});

/**
 * App-tab child webviews are hosted by WorkspaceLayout (routes `/` and
 * `/board`). Leaving those routes must hide every app webview so a web page
 * can never cover the settings/costs/... pages; coming back re-shows the
 * previously active tab. A global guard is used instead of a per-component
 * `watch(route.path)` because that component is unmounted during the
 * navigation and its watcher may never fire.
 */
const WORKSPACE_PATHS = new Set(["/", "/board"]);

/**
 * Route names of interest mapped to stable telemetry page ids. Only pages
 * listed here are reported, keeping `page_view` data focused on what we
 * actually want to measure (main workspace, session board, settings pages).
 */
const PAGE_VIEW_IDS: Record<string, string> = {
  workspace: "workspace",
  board: "board",
  "settings-agents": "settings-agents",
  "settings-apps": "settings-apps",
  "settings-models": "settings-models",
  "settings-models-commercial": "settings-models-commercial",
  "settings-general": "settings-general",
  "settings-costs": "settings-costs",
  "settings-skills": "settings-skills",
  "settings-about": "settings-about",
};

router.afterEach((to) => {
  const appTabs = useAppTabsStore();
  if (WORKSPACE_PATHS.has(to.path)) {
    appTabs.restore();
  } else {
    appTabs.hideAll();
  }
  // Telemetry: page view (fire-and-forget, never blocks navigation).
  const page = typeof to.name === "string" ? PAGE_VIEW_IDS[to.name] : undefined;
  if (page) track("page_view", { page });
});

export default router;
