# Homebrew formula for the austinintelligence/tap tap. Copy to Formula/android-use.rb in that repo.
# The sha256 values come from the release's SHA256SUMS file.
class AndroidUse < Formula
  desc "Give AI agents fast, bounded access to one Android device"
  homepage "https://github.com/austinintelligence/android-use"
  version "1.1.2"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/austinintelligence/android-use/releases/download/v#{version}/android-use-v#{version}-macos-aarch64.tar.gz"
      sha256 "REPLACE_WITH_SHA256_FROM_SHA256SUMS"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/austinintelligence/android-use/releases/download/v#{version}/android-use-v#{version}-linux-x86_64.tar.gz"
      sha256 "REPLACE_WITH_SHA256_FROM_SHA256SUMS"
    end
  end

  def install
    bin.install "au"
    pkgshare.install "aubridge.apk"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/au version")
  end
end
