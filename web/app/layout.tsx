import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "XLStatus",
  description: "Self-hosted server monitoring and operations system",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en" className="h-full antialiased">
      <body className="min-h-full flex flex-col">{children}</body>
    </html>
  );
}
