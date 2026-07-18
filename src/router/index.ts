import { createRouter, createWebHistory } from "vue-router";
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
      path: "/landing",
      name: "landing",
      component: () => import("../views/LandingPage.vue"),
    },
    {
      path: "/costs",
      name: "costs",
      component: () => import("../views/CostsView.vue"),
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
          path: "models",
          name: "settings-models",
          component: () => import("../views/settings/ModelsSettings.vue"),
        },
        {
          path: "general",
          name: "settings-general",
          component: () => import("../views/settings/GeneralSettings.vue"),
        },
      ],
    },
  ],
});

export default router;
