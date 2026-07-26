import { ReactNode } from "react";
import DashboardIdentityBoundary from "@/app/components/DashboardIdentityBoundary";
import Navigation from "@/app/components/Navigation";
import { ServerDataProvider } from "@/app/components/ServerDataProvider";
import { Toaster } from "@/app/components/Toasts";

// Shared chrome for all dashboard routes. Navigation was previously rendered
// by every page individually inside its own `min-h-screen` wrapper; hoisting it
// here removes that duplication and guarantees one consistent nav instance.
export default function DashboardLayout({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen">
      <DashboardIdentityBoundary>
        <ServerDataProvider>
          <Navigation />
          {children}
          <Toaster />
        </ServerDataProvider>
      </DashboardIdentityBoundary>
    </div>
  );
}
