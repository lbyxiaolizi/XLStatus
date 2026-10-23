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
          {`(function(){function isSafeCssVar(k,v){if(typeof k!=='string'||typeof v!=='string')return false;var tk=k.trim(),tv=v.trim();if(!/^--[a-zA-Z0-9_-]{1,128}$/.test(tk)||!tv||tv.length>256)return false;if(/[\\u0000-\\u001f\\u007f-\\u009f;{}<>\\\\'"\`@]/.test(tv))return false;if(tv.indexOf('/*')!==-1||tv.indexOf('*/')!==-1)return false;if(/(?:url\\s*\\(|expression\\s*\\(|javascript\\s*:|vbscript\\s*:|data\\s*:|-moz-binding|behavior\\s*:)/i.test(tv))return false;var d=0;for(var i=0;i<tv.length;i++){var c=tv.charAt(i);if(c==='(')d++;else if(c===')'){d--;if(d<0)return false;}}return d===0;}function applyCachedDashboardTheme(isDark){try{var root=document.documentElement;var body=document.body;var sections=['/dashboard','/servers','/services','/tasks','/terminal','/alerts','/notifications','/nat','/ddns','/tokens','/users','/mcp','/settings'];var dashboardPath=sections.some(function(prefix){return location.pathname===prefix||location.pathname.indexOf(prefix+'/')===0;});if(!dashboardPath)return;var user=JSON.parse(localStorage.getItem('user')||'null');var cached=JSON.parse(localStorage.getItem('xlstatus:dashboard-theme:v1')||'null');if(!user||!cached||cached.userId!==user.id||!cached.theme)return;var previous=(root.dataset.xlstatusDashboardThemeVars||'').split(',').filter(Boolean);previous.forEach(function(key){root.style.removeProperty(key);if(body)body.style.removeProperty(key);});var theme=cached.theme;var base=theme.variables||{};var light=Object.keys(theme.light_variables||{}).length?theme.light_variables:base;var dark=Object.keys(theme.dark_variables||{}).length?theme.dark_variables:light;var active=Object.assign({},base,isDark?dark:light);var keys=[];Object.keys(active).forEach(function(key){var value=active[key];if(!isSafeCssVar(key,value))return;var tk=key.trim(),tv=value.trim();root.style.setProperty(tk,tv);if(body)body.style.setProperty(tk,tv);keys.push(tk);});root.dataset.xlstatusDashboardThemeId=theme.id||'';root.dataset.xlstatusDashboardThemeVars=keys.join(',');}catch(error){localStorage.removeItem('xlstatus:dashboard-theme:v1');}}function applyTheme(){var isDark=localStorage.getItem('darkMode')==='true';var meta=document.getElementById('color-scheme-meta');if(isDark){document.documentElement.classList.add('dark-mode');if(document.body)document.body.classList.add('dark-mode');if(meta)meta.setAttribute('content','dark');}else{document.documentElement.classList.remove('dark-mode');if(document.body)document.body.classList.remove('dark-mode');if(meta)meta.setAttribute('content','light');}applyCachedDashboardTheme(isDark);}window.applyBoldTheme=applyTheme;applyTheme();})();`}
        </Script>
        {children}
      </body>
    </html>
  );
}
