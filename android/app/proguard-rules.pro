# The Rust core calls nothing in Kotlin, but Kotlin's native methods must keep
# their names: JNI looks them up as Java_io_manwe_toolnet_Core_<name>
-keepclasseswithmembernames class io.manwe.toolnet.Core {
    native <methods>;
}
