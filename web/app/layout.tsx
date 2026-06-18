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
          {`(function(){function applyTheme(){var isDark=localStorage.getItem('darkMode')==='true';var meta=document.getElementById('color-scheme-meta');if(isDark){document.documentElement.classList.add('dark-mode');if(document.body)document.body.classList.add('dark-mode');if(meta)meta.setAttribute('content','dark');}else{document.documentElement.classList.remove('dark-mode');if(document.body)document.body.classList.remove('dark-mode');if(meta)meta.setAttribute('content','light');}}window.applyBoldTheme=applyTheme;applyTheme();})();`}
        </Script>
        {children}
      </body>
    </html>
  );
}
