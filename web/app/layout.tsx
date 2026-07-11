import type { Metadata } from "next";
import Script from "next/script";
import { defaultLocale, t } from "@/lib/i18n";
import "./globals.css";

export const metadata: Metadata = {
  title: "XLStatus",
  description: t.appDescription,
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang={defaultLocale} className="h-full antialiased" suppressHydrationWarning>
      <head>
        <meta name="color-scheme" id="color-scheme-meta" content="light" />
      </head>
      <body className="min-h-full flex flex-col" suppressHydrationWarning>
        <Script id="bold-theme-init" strategy="beforeInteractive">
          {`(function(){function applyCachedDashboardTheme(isDark){try{var root=document.documentElement;var body=document.body;var sections=['/dashboard','/servers','/services','/tasks','/terminal','/alerts','/notifications','/nat','/ddns','/settings'];var dashboardPath=sections.some(function(prefix){return location.pathname===prefix||location.pathname.indexOf(prefix+'/')===0;});if(!dashboardPath)return;var user=JSON.parse(localStorage.getItem('user')||'null');var cached=JSON.parse(localStorage.getItem('xlstatus:dashboard-theme:v1')||'null');if(!user||!cached||cached.userId!==user.id||!cached.theme)return;var previous=(root.dataset.xlstatusDashboardThemeVars||'').split(',').filter(Boolean);previous.forEach(function(key){root.style.removeProperty(key);if(body)body.style.removeProperty(key);});var theme=cached.theme;var base=theme.variables||{};var light=Object.keys(theme.light_variables||{}).length?theme.light_variables:base;var dark=Object.keys(theme.dark_variables||{}).length?theme.dark_variables:light;var active=Object.assign({},base,isDark?dark:light);var keys=[];Object.keys(active).forEach(function(key){var value=active[key];if(key.indexOf('--')!==0||typeof value!=='string'||!value)return;root.style.setProperty(key,value);if(body)body.style.setProperty(key,value);keys.push(key);});root.dataset.xlstatusDashboardThemeId=theme.id||'';root.dataset.xlstatusDashboardThemeVars=keys.join(',');}catch(error){localStorage.removeItem('xlstatus:dashboard-theme:v1');}}function applyTheme(){var isDark=localStorage.getItem('darkMode')==='true';var meta=document.getElementById('color-scheme-meta');if(isDark){document.documentElement.classList.add('dark-mode');if(document.body)document.body.classList.add('dark-mode');if(meta)meta.setAttribute('content','dark');}else{document.documentElement.classList.remove('dark-mode');if(document.body)document.body.classList.remove('dark-mode');if(meta)meta.setAttribute('content','light');}applyCachedDashboardTheme(isDark);}window.applyBoldTheme=applyTheme;applyTheme();})();`}
        </Script>
        {children}
      </body>
    </html>
  );
}
