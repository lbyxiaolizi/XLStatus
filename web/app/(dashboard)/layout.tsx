import { ReactNode } from "react";
import Navigation from "@/app/components/Navigation";

// Shared chrome for all dashboard routes. Navigation was previously rendered
// by every page individually inside its own `min-h-screen` wrapper; hoisting it
// here removes that duplication and guarantees one consistent nav instance.
export default function DashboardLayout({ children }: { children: ReactNode }) {
  return (
    <div className="min-h-screen">
      <Navigation />
      {children}
    </div>
  );
}
