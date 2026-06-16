'use client';

import { useState } from 'react';
import Navigation from '@/app/components/Navigation';

export default function AlertsPage() {
  const [alerts] = useState([]);

  return (
    <div>
      <Navigation />
      <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        <div className="mb-6 flex justify-between items-center">
          <h1 className="text-2xl font-bold text-gray-900">Alert Rules</h1>
          <button
            className="bg-blue-600 hover:bg-blue-700 text-white font-medium py-2 px-4 rounded"
          >
            Create Alert Rule
          </button>
        </div>

        <div className="text-center py-12 bg-white rounded-lg shadow">
          <p className="text-gray-500">No alert rules configured</p>
          <p className="text-sm text-gray-400 mt-2">
            Create alert rules to get notified when issues occur
          </p>
        </div>
      </div>
    </div>
  );
}
