"use client";

import { PageHeader, PageShell } from "@/app/components/M7Primitives";
import { useI18n } from "@/lib/use-i18n";
import { AgentInstallCard } from "./sections/AgentInstallCard";
import { CloudflaredCard } from "./sections/CloudflaredCard";
import { GeoIpCard } from "./sections/GeoIpCard";
import { MaintenanceCard } from "./sections/MaintenanceCard";
import { PatTokensCard } from "./sections/PatTokensCard";
import { PublicSiteCard } from "./sections/PublicSiteCard";
import { SecurityCard } from "./sections/SecurityCard";
import { SessionsCard } from "./sections/SessionsCard";
import { ThemesCard } from "./sections/ThemesCard";
import { UsersCard } from "./sections/UsersCard";
import { WafCard } from "./sections/WafCard";

// Every card owns its state, dialogs, feedback and data loading, so typing in
// one card cannot re-render the rest of the page.
export default function SettingsPage() {
  const { t: copy } = useI18n();

  return (
    <PageShell>
      <PageHeader
        eyebrow={copy.settingsPage.eyebrow}
        title={copy.settingsPage.title}
        detail={copy.settingsPage.detail}
      />
      <AgentInstallCard />
      <ThemesCard />
      <CloudflaredCard />
      <SecurityCard />
      <PublicSiteCard />
      <UsersCard />
      <SessionsCard />
      <WafCard />
      <MaintenanceCard />
      <GeoIpCard />
      <PatTokensCard />
    </PageShell>
  );
}
