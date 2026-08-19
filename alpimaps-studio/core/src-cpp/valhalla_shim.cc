// A C boundary over valhalla::tyr::actor_t.
//
// actor_t is the same embedding path the mobile bindings use: JSON in, JSON out, no HTTP server
// and therefore no prime_server or zmq. Everything crossing this boundary is a C string, so the
// Rust side needs no C++ ABI knowledge.
//
// Every entry point catches, because a Valhalla exception unwinding into Rust is undefined
// behaviour rather than an error.

#include <valhalla/tyr/actor.h>
#include <boost/property_tree/json_parser.hpp>
#include <boost/property_tree/ptree.hpp>

#include <cstdlib>
#include <cstring>
#include <exception>
#include <sstream>
#include <string>

namespace {

char* copy_out(const std::string& text) {
  char* out = static_cast<char*>(std::malloc(text.size() + 1));
  if (out != nullptr) {
    std::memcpy(out, text.c_str(), text.size() + 1);
  }
  return out;
}

void set_error(char** error, const std::string& message) {
  if (error != nullptr) {
    *error = copy_out(message);
  }
}

} // namespace

extern "C" {

/// Build an actor from a Valhalla config JSON string. Returns null and sets `error` on failure.
void* valhalla_actor_create(const char* config_json, char** error) {
  try {
    boost::property_tree::ptree config;
    std::stringstream stream(config_json);
    boost::property_tree::read_json(stream, config);
    return new valhalla::tyr::actor_t(config, true);
  } catch (const std::exception& e) {
    set_error(error, e.what());
    return nullptr;
  } catch (...) {
    set_error(error, "unknown error creating valhalla actor");
    return nullptr;
  }
}

/// Run a route request. Caller frees the result with `valhalla_string_free`.
char* valhalla_actor_route(void* handle, const char* request_json, char** error) {
  auto* actor = static_cast<valhalla::tyr::actor_t*>(handle);
  if (actor == nullptr) {
    set_error(error, "null actor");
    return nullptr;
  }
  try {
    return copy_out(actor->route(request_json));
  } catch (const std::exception& e) {
    set_error(error, e.what());
    return nullptr;
  } catch (...) {
    set_error(error, "unknown routing error");
    return nullptr;
  }
}

void valhalla_actor_destroy(void* handle) {
  delete static_cast<valhalla::tyr::actor_t*>(handle);
}

void valhalla_string_free(char* text) {
  std::free(text);
}

} // extern "C"
