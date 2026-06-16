export default function Home() {
  return (
    <div className="min-h-screen flex items-center justify-center bg-gray-50">
      <div className="text-center">
        <h1 className="text-4xl font-bold text-gray-900 mb-4">
          XLStatus
        </h1>
        <p className="text-lg text-gray-600 mb-8">
          Self-hosted server monitoring and operations system
        </p>
        <div className="space-x-4">
          <a
            href="/login"
            className="inline-block px-6 py-3 bg-blue-600 text-white rounded-lg hover:bg-blue-700"
          >
            Sign In
          </a>
          <a
            href="/dashboard"
            className="inline-block px-6 py-3 bg-gray-200 text-gray-800 rounded-lg hover:bg-gray-300"
          >
            Dashboard
          </a>
        </div>
        <div className="mt-8 text-sm text-gray-500">
          <p>M1 Base Platform Complete</p>
          <p className="mt-2">✅ Authentication • ✅ PAT System • ✅ RBAC</p>
        </div>
      </div>
    </div>
  );
}
