// Alegria SEO static site: isolated from CRM staging and all existing routes.
// Only publish real expert-approved generated artifacts; never ship fixtures.
// The dedicated account workers.dev URL is the only route; no custom DNS.
import { defineConfig } from "cf/config";

export default defineConfig({
  accountId: "78e5fcb3762c1473113d51838556da74",
  worker: {
    name: "alegria-seo-staging",
    compatibilityDate: "2026-11-08",
    workersDev: true,
    assets: {
      htmlHandling: "auto-trailing-slash",
      notFoundHandling: "none",
    },
  },
});
