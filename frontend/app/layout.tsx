import type { Metadata } from "next";
import { Geist, Geist_Mono } from "next/font/google";
import "./globals.css";

const geistSans = Geist({
  variable: "--font-geist-sans",
  subsets: ["latin"],
});

const geistMono = Geist_Mono({
  variable: "--font-geist-mono",
  subsets: ["latin"],
});

export const metadata: Metadata = {
  title: "Seasoned Hand",
  description: "An autonomous AI employee that gets seasoned by the work you delegate.",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html
      lang="en"
      className={`${geistSans.variable} ${geistMono.variable} h-full antialiased`}
    >
      {/* Body height MUST be definite (`h-full` = height:100%) so child
          elements with `height: 100%` (e.g. react-resizable-panels v4's
          Group inline style) resolve against a known size. The previous
          `min-h-full` made body indefinite-height under flex layout, which
          collapsed the 3-panel Group to its content height (277 px) instead
          of filling the viewport. */}
      <body className="h-full">{children}</body>
    </html>
  );
}
