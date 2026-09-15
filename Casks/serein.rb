cask "serein" do
  version "1.0.0-nightly.20260915.23"
  sha256 "2b3b4373e7f5abf9d7495716146ec8bff071e7ab7d225d97e967238f65976de7"

  url "https://github.com/ViceVerse-cz/Serein/releases/download/v#{version}/serein-v#{version}-macOS-ARM64.zip"
  name "Serein"
  desc "Experimental native Discord client"
  homepage "https://github.com/ViceVerse-cz/Serein"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Serein.app"
end
