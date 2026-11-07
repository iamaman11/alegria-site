// Alegria SEO static site: isolated from CRM staging and all existing routes.
// Only publish real expert-approved generated artifacts; never ship fixtures.
// The assets-only deployment directory is supplied to: cf deploy <release-dir>.
import { defineConfig } from "cf/config";

export default defineConfig({
  accountId: "4426df1449e417511bc7697d60b7f62f",
  worker: {
    name: "alegria-seo-staging",
    compatibilityDate: "2026-11-08",
    domains: ["seo-staging.alegria.by"],
    assets: {
      htmlHandling: "auto-trailing-slash",
      notFoundHandling: "none",
    },
  },
});
