import { createRouter, createWebHistory } from "vue-router";
import { useAppTabsStore } from "../stores/useAppTabsStore";
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
      path: "/costs",
      name: "costs",
      component: () => import("../views/CostsView.vue"),
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

router.afterEach((to) => {
  const appTabs = useAppTabsStore();
  if (WORKSPACE_PATHS.has(to.path)) {
    appTabs.restore();
  } else {
    appTabs.hideAll();
  }
});

export default router;
