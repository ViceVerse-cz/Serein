cask "serein" do
  version "1.0.0-nightly.20260921.44"
  sha256 "e7021c5dbd031076644426856c5fea6cc2b33ab7a7df64a963a1e993712544ab"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
