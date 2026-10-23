"use client";

import { useState } from "react";
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

type SettingsTabId = "access" | "appearance" | "security" | "network" | "maintenance" | "all";

export default function SettingsPage() {
  const { locale, t: copy } = useI18n();
  const [activeTab, setActiveTab] = useState<SettingsTabId>("access");
  const isZh = locale === "zh-CN";

  const categories: Array<{
    id: SettingsTabId;
    label: string;
    subtitle: string;
    badge: string;
  }> = [
    {
      id: "access",
      label: isZh ? "节点接入与令牌" : "Nodes & Tokens",
      subtitle: isZh ? "Agent 一键安装脚本、注册令牌与 PAT" : "Agent install script, enrollment & PAT",
      badge: "2",
    },
    {
      id: "appearance",
      label: isZh ? "外观与公开站点" : "Appearance & Public",
      subtitle: isZh ? "公开状态页开关、品牌信息与主题模板" : "Public status page, branding & themes",
      badge: "2",
    },
    {
      id: "security",
      label: isZh ? "账户与访问安全" : "Security & IAM",
      subtitle: isZh ? "TOTP 双因素、OAuth、用户、会话与 WAF" : "TOTP 2FA, OAuth, users, sessions & WAF",
      badge: "4",
    },
    {
      id: "network",
      label: isZh ? "网络与解析服务" : "Network & GeoIP",
      subtitle: isZh ? "Cloudflare Tunnel 隧道、GeoIP 与 DoH" : "Cloudflare Tunnel, GeoIP & DoH resolver",
      badge: "2",
    },
    {
      id: "maintenance",
      label: isZh ? "数据备份与维护" : "Backup & TSDB",
      subtitle: isZh ? "SQLite 备份恢复、Vacuum 与 TSDB 压缩" : "SQLite backup/restore, vacuum & TSDB",
      badge: "1",
    },
    {
      id: "all",
      label: isZh ? "全部模块平铺" : "All Sections",
      subtitle: isZh ? "在同一视图内展开所有设置卡片" : "Show all settings modules on one page",
      badge: "11",
    },
  ];

  const currentCategory = categories.find((item) => item.id === activeTab) ?? categories[0];

  return (
    <PageShell wide>
      <PageHeader
        eyebrow={copy.settingsPage.eyebrow}
        title={copy.settingsPage.title}
        detail={copy.settingsPage.detail}
      />

      <div className="grid gap-6 lg:grid-cols-[16.5rem_minmax(0,1fr)] lg:items-start">
        {/* Left Sidebar Navigation on Desktop / Horizontal Scroll Pills on Mobile */}
        <aside className="lg:sticky lg:top-20">
          <div className="rounded-xl border border-[var(--border-color)] bg-[var(--bg-card)] p-2 shadow-[var(--shadow-xs)]">
            <div className="mb-1.5 hidden px-2.5 pt-1 text-[11px] font-semibold tracking-wider text-[var(--text-faint)] uppercase lg:block">
              {isZh ? "配置分组" : "Categories"}
            </div>
            <div className="flex gap-1.5 overflow-x-auto lg:grid lg:gap-1">
              {categories.map((cat) => {
                const active = cat.id === activeTab;
                return (
                  <button
                    key={cat.id}
                    type="button"
                    onClick={() => setActiveTab(cat.id)}
                    className={`group flex shrink-0 items-start justify-between gap-2.5 rounded-lg px-3 py-2.5 text-left transition ${
                      active
                        ? "bg-[var(--bg-muted)] text-[var(--text-main)] shadow-[var(--shadow-xs)]"
                        : "text-[var(--text-muted)] hover:bg-[var(--bg-subtle)] hover:text-[var(--text-main)]"
                    }`}
                  >
                    <div className="min-w-0">
                      <div className="flex items-center gap-2">
                        <span
                          className={`h-1.5 w-1.5 shrink-0 rounded-full transition ${
                            active ? "bg-[var(--accent-color)]" : "bg-transparent group-hover:bg-[var(--border-strong)]"
                          }`}
                        />
                        <span className={`text-xs ${active ? "font-semibold" : "font-medium"}`}>
                          {cat.label}
                        </span>
                      </div>
                      <p className="mt-0.5 hidden pl-3.5 text-[11px] leading-snug text-[var(--text-muted)] lg:block">
                        {cat.subtitle}
                      </p>
                    </div>
                    <span
                      className={`rounded-full border px-1.5 py-0.2 font-mono text-[10px] ${
                        active
                          ? "border-[var(--border-strong)] bg-[var(--bg-card)] text-[var(--text-main)]"
                          : "border-[var(--border-color)] bg-[var(--bg-subtle)] text-[var(--text-faint)]"
                      }`}
                    >
                      {cat.badge}
                    </span>
                  </button>
                );
              })}
            </div>
          </div>
        </aside>

        {/* Active Settings Workspace */}
        <div className="min-w-0 space-y-6">
          {currentCategory ? (
            <div className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-[var(--border-color)] bg-[var(--bg-subtle)] px-4 py-3">
              <div>
                <h2 className="text-sm font-semibold text-[var(--text-main)]">
                  {currentCategory.label}
                </h2>
                <p className="text-xs text-[var(--text-muted)]">{currentCategory.subtitle}</p>
              </div>
              <span className="rounded-full border border-[var(--border-color)] bg-[var(--bg-card)] px-2.5 py-0.5 font-mono text-xs text-[var(--text-muted)]">
                {currentCategory.badge} {isZh ? "个模块" : "modules"}
              </span>
            </div>
          ) : null}

          {(activeTab === "access" || activeTab === "all") && (
            <>
              <AgentInstallCard />
              <PatTokensCard />
            </>
          )}

          {(activeTab === "appearance" || activeTab === "all") && (
            <>
              <PublicSiteCard />
              <ThemesCard />
            </>
          )}

          {(activeTab === "security" || activeTab === "all") && (
            <>
              <SecurityCard />
              <UsersCard />
              <SessionsCard />
              <WafCard />
            </>
          )}

          {(activeTab === "network" || activeTab === "all") && (
            <>
              <CloudflaredCard />
              <GeoIpCard />
            </>
          )}

          {(activeTab === "maintenance" || activeTab === "all") && <MaintenanceCard />}
        </div>
      </div>
    </PageShell>
  );
}
