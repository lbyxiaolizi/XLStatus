'use client';

import { useEffect, useState } from 'react';

interface ServiceStatus {
  name: string;
  status: 'operational' | 'degraded' | 'down';
  uptime: number;
}

export default function StatusPage() {
  const [services, setServices] = useState<ServiceStatus[]>([
    { name: 'Web Service', status: 'operational', uptime: 99.9 },
    { name: 'API Service', status: 'operational', uptime: 99.8 },
    { name: 'Database', status: 'operational', uptime: 100 },
  ]);

  const getStatusColor = (status: string) => {
    switch (status) {
      case 'operational':
        return 'bg-green-500';
      case 'degraded':
        return 'bg-yellow-500';
      case 'down':
        return 'bg-red-500';
      default:
        return 'bg-gray-500';
    }
  };

  const getStatusText = (status: string) => {
    switch (status) {
      case 'operational':
        return 'Operational';
      case 'degraded':
        return 'Degraded Performance';
      case 'down':
        return 'Down';
      default:
        return 'Unknown';
    }
  };

  const overallStatus = services.every(s => s.status === 'operational')
    ? 'All Systems Operational'
    : services.some(s => s.status === 'down')
    ? 'Service Disruption'
    : 'Partial Outage';

  return (
    <div className="min-h-screen bg-gray-50">
      {/* Header */}
      <div className="bg-white shadow">
        <div className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8 py-6">
          <h1 className="text-3xl font-bold text-gray-900">XLStatus</h1>
          <p className="text-sm text-gray-500 mt-1">System Status</p>
        </div>
      </div>

      <div className="max-w-4xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        {/* Overall Status */}
        <div className="bg-white shadow rounded-lg p-6 mb-6">
          <div className="flex items-center">
            <div className={`w-3 h-3 rounded-full ${
              overallStatus === 'All Systems Operational'
                ? 'bg-green-500'
                : overallStatus === 'Service Disruption'
                ? 'bg-red-500'
                : 'bg-yellow-500'
            }`} />
            <h2 className="ml-3 text-xl font-semibold text-gray-900">
              {overallStatus}
            </h2>
          </div>
          <p className="mt-2 text-sm text-gray-600">
            Last updated: {new Date().toLocaleString()}
          </p>
        </div>

        {/* Services */}
        <div className="bg-white shadow rounded-lg divide-y divide-gray-200">
          {services.map((service, index) => (
            <div key={index} className="p-6">
              <div className="flex items-center justify-between">
                <div className="flex items-center">
                  <div className={`w-3 h-3 rounded-full ${getStatusColor(service.status)}`} />
                  <div className="ml-3">
                    <h3 className="text-base font-medium text-gray-900">
                      {service.name}
                    </h3>
                    <p className="text-sm text-gray-500">
                      {getStatusText(service.status)}
                    </p>
                  </div>
                </div>
                <div className="text-right">
                  <p className="text-sm font-medium text-gray-900">
                    {service.uptime}% uptime
                  </p>
                  <p className="text-xs text-gray-500">Last 30 days</p>
                </div>
              </div>
            </div>
          ))}
        </div>

        {/* Footer */}
        <div className="mt-8 text-center text-sm text-gray-500">
          <p>Powered by XLStatus</p>
          <p className="mt-1">
            <a href="/login" className="text-blue-600 hover:text-blue-800">
              Admin Login
            </a>
          </p>
        </div>
      </div>
    </div>
  );
}
